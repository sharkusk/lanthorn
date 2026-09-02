//! SQ-0844: **a multi-disk release is one collection**, measured on all three
//! compilation families in the real corpus.
//!
//! Two defects, one idea. Naming `disk1.img` opened whatever single story that
//! one image held and left the eleven games on its siblings reachable only by
//! naming each sibling in turn; naming *The Lost Treasures of Infocom*'s disk 1
//! failed outright, because the Apple II press puts a launcher there and no
//! story at all. And in the browser the nine Atari ST volumes offered 39 rows
//! for 33 games, because `Infocom Compilation 8` is four second copies of builds
//! `Compilation 1` and `Compilation 5` already carry.
//!
//! The rule under test lives in [`app::disk_set`] (name-only, unit-tested there)
//! and is applied in two places: [`app::picker::scan_stories`] folds a set's
//! duplicate builds, and [`app::picker::StorySource::of`] turns a named volume
//! into the whole release.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! when its fixture is missing and every `ran > 0` guard is gated on
//! [`any_media_present`] — CI has none of it on any platform and must not fail
//! on its absence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One multi-disk release in the corpus: a member to name, how many volumes the
/// rule must find, and how many games the set offers once folded.
struct Set {
    /// Any one volume — deliberately not always disk 1.
    member: &'static str,
    volumes: usize,
    /// Rows the set's volumes produce before folding.
    raw: usize,
    /// Rows after folding duplicate builds — the number of games it offers.
    games: usize,
}

/// Measured 2026-08-14 on the local corpus. Each row is a fact about the media,
/// not about the code: `raw` is what the mounts report and `games` is the count
/// of distinct IFIDs among them.
const SETS: &[Set] = &[
    // The Atari ST shelf — the only family with duplicates, and the reason
    // dedupe exists. Named by disk 5, which is where Trinity's first copy is.
    Set { member: "Infocom Compilation 5 (19xx)(-).st", volumes: 9, raw: 39, games: 33 },
    // The Apple II press. Named by disk 1, which holds a LAUNCHER and no story:
    // before this quest that argument was an error message.
    Set {
        member: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 1 of 7).2mg",
        volumes: 7,
        raw: 30,
        games: 30,
    },
    // The two DOS families, which must stay two sets.
    Set { member: "floppy3.ima", volumes: 5, raw: 20, games: 20 },
    Set { member: "disk1.img", volumes: 4, raw: 11, games: 11 },
];

/// Release floppies that are **not** part of any set, and must not become one.
const SINGLE_TITLE: &[&str] = &[
    "Zork I - The Great Underground Empire.adf",
    "Zork II - The Wizard of Frobozz.adf",
    "Zork III - The Dungeon Master.adf",
    "Zork Zero - The Revenge of Megaboz.adf",
    "Journey - The Quest Begins.adf",
    "Arthur - The Quest for Excalibur.adf",
    "Zork Zero Disk.image",
    "Beyond Zork (1988)(Infocom).2mg",
    // A stem with a bare digit in it and no sibling to pair with.
    "Arthur Quest 4 Excalibur.2mg",
    // A `(360K)` in the stem, one disk, no set.
    "Hitchhiker's Guide to the Galaxy, The (1987) (r58, Serial 851002) (Infocom, Inc.) (360K) [!].ima",
];

fn any_media_present() -> bool {
    SETS.iter()
        .map(|s| s.member)
        .chain(SINGLE_TITLE.iter().copied())
        .any(|f| stories_dir().join(f).exists())
}

fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq0844-{tag}"))
}

/// What the mounts themselves say is on `paths` — the answer the picker must
/// match, taken from `blorb` rather than from the code under test.
fn ifids_on(paths: &[PathBuf]) -> Vec<(PathBuf, String, String)> {
    let mut out = Vec::new();
    for p in paths {
        let Ok(raw) = std::fs::read(p) else { continue };
        if blorb::medium::DiskImage::detect(&raw).is_none() {
            continue;
        }
        let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { continue };
        for s in disk.stories() {
            out.push((p.clone(), s.name, app::ifid::compute_ifid(&s.bytes)));
        }
    }
    out
}

// ── Discovery: one volume brings in the release ──────────────────────────────

/// **The user's requirement.** Naming any one volume finds the whole set.
#[test]
fn naming_one_volume_finds_the_whole_release() {
    let mut ran = 0;
    for set in SETS {
        let path = stories_dir().join(set.member);
        if !path.is_file() {
            continue;
        }
        ran += 1;
        let members = app::disk_set::members(&path)
            .unwrap_or_else(|| panic!("{}: named a volume and got no set", set.member));
        assert_eq!(members.len(), set.volumes, "{}: members {members:?}", set.member);
        assert!(members.contains(&path), "{}: a volume is not in its own set", set.member);
        // Ordered by disk number, so "the first disk" is a fact and not a guess.
        assert!(members.windows(2).all(|w| w[0] <= w[1]) || members.len() < 2);
        // Every member is a real, mountable disk image — the rule reads names,
        // but the names it accepted had better be disks.
        for m in &members {
            let raw = std::fs::read(m).expect("a member exists");
            assert!(
                blorb::medium::DiskImage::detect(&raw).is_some(),
                "{}: {m:?} was grouped in but is not a disk image",
                set.member,
            );
        }
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no set was discovered");
}

/// The other half of discovery: **a single-title floppy is not a set**, which is
/// most of the corpus and the path that must not move.
#[test]
fn a_single_title_floppy_is_not_a_set() {
    let mut ran = 0;
    for name in SINGLE_TITLE {
        let path = stories_dir().join(name);
        if !path.is_file() {
            continue;
        }
        ran += 1;
        assert!(
            app::disk_set::members(&path).is_none(),
            "{name}: a lone release floppy was read as a volume of a set",
        );
        assert!(
            app::picker::StorySource::of(&path, Path::new("/nonexistent")).is_none(),
            "{name}: naming it would open a browser instead of the game",
        );
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no single title ran");
}

/// A loose story file is never a set, whatever its name looks like — including
/// `adv01.dat`…`adv13.dat`, thirteen unrelated Scott Adams games that are a
/// textbook prefix-plus-index and the corpus's best false-positive bait.
#[test]
fn numbered_loose_story_files_are_not_a_set() {
    let dir = stories_dir();
    if !dir.is_dir() {
        return;
    }
    let mut ran = 0;
    for n in 1..=13 {
        let path = dir.join(format!("adv{n:02}.dat"));
        if !path.is_file() {
            continue;
        }
        ran += 1;
        assert!(app::disk_set::members(&path).is_none(), "adv{n:02}.dat was grouped into a set");
    }
    assert!(ran > 0 || !dir.join("adv01.dat").exists());
}

/// **The corpus's sharpest false positive, on the real files.** Zork Zero's DOS
/// 360K and 720K presses spell their disks alike, so `(360K) (Disk 1)` and
/// `(720K) (Disk 1)` differ at exactly one digit run — `{360, 720}`, a capacity.
/// They must come out as two sets, never one.
#[test]
fn the_two_zork_zero_dos_presses_are_two_sets() {
    let dir = stories_dir();
    let base = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.)";
    let k360 = dir.join(format!("{base} (360K) (Disk 1) [!].ima"));
    let k720 = dir.join(format!("{base} (720K) (Disk 1) [!].ima"));
    if !k360.is_file() || !k720.is_file() {
        return;
    }
    let a = app::disk_set::members(&k360).expect("the 360K press is a set");
    let b = app::disk_set::members(&k720).expect("the 720K press is a set");
    assert_eq!(a.len(), 3, "the 360K press is three disks: {a:?}");
    assert_eq!(b.len(), 2, "the 720K press is two disks: {b:?}");
    for m in &a {
        assert!(
            m.to_string_lossy().contains("360K"),
            "a 720K disk was folded into the 360K press: {m:?}",
        );
        assert!(!b.contains(m), "the two presses overlap at {m:?}");
    }
}

/// `disk*.img` and `floppy*.ima` are two DOS families in one directory and two
/// sets — the case the user named when asking for this.
#[test]
fn the_two_dos_families_are_two_sets() {
    let dir = stories_dir();
    let (d, f) = (dir.join("disk1.img"), dir.join("floppy1.ima"));
    if !d.is_file() || !f.is_file() {
        return;
    }
    let ds = app::disk_set::members(&d).expect("disk*.img is a set");
    let fs = app::disk_set::members(&f).expect("floppy*.ima is a set");
    assert!(ds.iter().all(|p| !fs.contains(p)), "the two DOS families were merged");
    assert!(ds.iter().all(|p| p.extension().unwrap() == "img"));
    assert!(fs.iter().all(|p| p.extension().unwrap() == "ima"));
}

// ── Presentation: the set is its games ───────────────────────────────────────

/// Naming a volume opens the release's **complete menu of games** — the user's
/// request in one assertion. A set is presented as its games, one row each, and
/// never as its disks: SQ-0859's shape, extended across volumes.
///
/// FALSIFICATION (measured 2026-08-14, with `StorySource::of` returning `None`
/// for a `DiskSet`, i.e. the pre-SQ-0844 single-file launch):
///
/// ```text
/// thread 'disk_set_rows::naming_a_volume_offers_the_whole_release' panicked at
/// crates/app/tests/suites/disk_set_rows.rs:251:13:
/// Infocom Compilation 5 (19xx)(-).st: naming a volume offered no set at all — the release was never assembled
/// ```
///
/// Three of this suite's neighbours failed in the same run —
/// `the_apple_ii_launcher_disk_opens_the_collection`,
/// `a_build_carried_by_two_volumes_is_one_row` and
/// `a_releases_games_keep_their_own_saves` — each on the same missing set.
#[test]
fn naming_a_volume_offers_the_whole_release() {
    let mut ran = 0;
    for set in SETS {
        let path = stories_dir().join(set.member);
        if !path.is_file() {
            continue;
        }
        let base = data_base("offer");
        ran += 1;
        let source = app::picker::StorySource::of(&path, &base).unwrap_or_else(|| {
            panic!("{}: naming a volume offered no set at all — the release was never assembled", set.member)
        });
        let rows = source.scan(&base);
        assert_eq!(
            rows.len(),
            set.games,
            "{}: the release offers {} games, the browser listed {}",
            set.member,
            set.games,
            rows.len(),
        );
        // Rows are STORIES, not disks: more rows than the set has volumes, and
        // no row is titled after the box it came in.
        assert!(rows.len() > set.volumes || set.games <= set.volumes);
        for r in &rows {
            let stem = r.path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            assert_ne!(r.title, stem, "{}: a row is titled after its disk", set.member);
        }
        // The games come off several volumes — that is what makes it a set.
        let sources: BTreeSet<&Path> = rows.iter().map(|r| r.path.as_path()).collect();
        assert!(sources.len() > 1, "{}: every row came off one disk", set.member);
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no release was offered");
}

/// **Disk 1 of the Apple II press holds a launcher and no story**, so naming it
/// used to be an error — *"no story file on the disk image … (is this the boot
/// disk?)"*. It is the most direct statement of what this quest is for.
#[test]
fn the_apple_ii_launcher_disk_opens_the_collection() {
    let path = stories_dir()
        .join("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 1 of 7).2mg");
    if !path.is_file() {
        return;
    }
    // It really does carry no story — the premise, checked rather than assumed.
    assert!(
        app::picker::resolve_entries(&path, &data_base("premise")).is_empty(),
        "the premise moved: disk 1 now carries a story of its own",
    );
    let base = data_base("launcher");
    let source = app::picker::StorySource::of(&path, &base).expect("disk 1 opens the collection");
    let rows = source.scan(&base);
    assert_eq!(rows.len(), 30, "the Lost Treasures shelf is 30 games");
    let _ = std::fs::remove_dir_all(&base);
}

/// A set whose games number fewer than two is not worth a menu: naming Zork
/// Zero's `(360K) (Disk 2)` opens Zork Zero, exactly as it always did, even
/// though the three-disk set around it is correctly recognised.
#[test]
fn a_set_offering_one_game_still_opens_that_game() {
    let path = stories_dir().join(
        "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 2) [!].ima",
    );
    if !path.is_file() {
        return;
    }
    let base = data_base("onegame");
    assert!(app::disk_set::members(&path).is_some(), "the set is still recognised");
    assert!(
        app::picker::StorySource::of(&path, &base).is_none(),
        "a one-game set must not put a browser in front of the game",
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// **And the other volumes of that one-game set open it too** (SQ-0941).
///
/// The DOS press keeps the story whole on one floppy and puts the installer and
/// the artwork on the others — so `(360K) (Disk 1)`, the disk with `INSTALL.EXE`
/// on it and the disk a player naturally opens first, was the one that could not
/// work. Measured on the three volumes of *Zork Zero* release 393 / serial
/// 890714 (three independent FAT12 filesystems, not one container paged across
/// them):
///
/// | volume | files | before |
/// | --- | --- | --- |
/// | Disk 1 | `INSTALL.EXE`, `EZR.EXE`, `IZORK0.RUN`, `ZORK0.CG1` | no story |
/// | Disk 2 | `ZORK0.ZIP`, `ZORKZERO.EXE` | the game |
/// | Disk 3 | `ZORK0.EG1` | no story |
///
/// The 720K press of the same build is the mirror: two volumes, the story on
/// disk 1 and CGA's plates alone on disk 2.
///
/// FALSIFICATION: drop `story_elsewhere_in_the_release` from
/// `cli_host::disk_set::mount_at` and every volume but the story's own fails
/// with `no story file on the disk image … (4 files on ZORK0 1; is this the boot
/// disk?)`.
#[test]
fn every_volume_of_a_one_game_set_opens_its_game() {
    // (the volume to name, whether the story is physically on it)
    const PRESSES: &[(&str, bool)] = &[
        ("(360K) (Disk 1)", false),
        ("(360K) (Disk 2)", true),
        ("(360K) (Disk 3)", false),
        ("(720K) (Disk 1)", true),
        ("(720K) (Disk 2)", false),
    ];
    let mut ran = 0;
    let mut off_a_sibling = 0;
    for (press, carries_it) in PRESSES {
        let path = stories_dir().join(format!(
            "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) {press} [!].ima"
        ));
        if !path.is_file() {
            continue;
        }
        ran += 1;
        // The premise, checked rather than assumed: only the story's own volume
        // answers when the set is not consulted.
        let raw = std::fs::read(&path).expect("the volume reads");
        let alone = blorb::medium::MountedDisk::mount(raw).expect("it mounts");
        assert_eq!(
            !alone.stories().is_empty(),
            *carries_it,
            "{press}: the premise moved — which volume physically carries the story",
        );
        if !carries_it {
            off_a_sibling += 1;
        }

        let (loaded, image) =
            app::hints::load_mounted_story(&path).unwrap_or_else(|e| panic!("{press}: {e}"));
        let bytes = loaded.bytes();
        assert_eq!(bytes[0], 6, "{press}: Zork Zero is a Version 6 story");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 393, "{press}: release");
        assert_eq!(&bytes[0x12..0x18], b"890714", "{press}: serial");
        // The medium comes from the mount that answered, and on a DOS press
        // every volume is the same FAT12 filesystem either way.
        assert_eq!(image, Some(blorb::medium::DiskImage::Fat12Dos), "{press}: medium");
    }
    // `ran == 0` is the gitignored-media skip; anything else must have exercised
    // at least one volume that does NOT carry the story, or nothing was proved.
    assert!(
        off_a_sibling > 0 || ran == 0,
        "every volume that ran already carried the story — nothing was proved",
    );
}

// ── Dedupe: the same build twice is one row ──────────────────────────────────

/// **The measured duplication.** Trinity, Lurking Horror, Moonmist, Stationfall,
/// Cutthroats and Hitchhiker's are each carried by two volumes of the Atari ST
/// shelf — `Compilation 5` stores its games flat (`TRINITY.T`) and `Compilation
/// 8` in per-game directories (`TRINITY/STORY.DAT`), the same build both times.
/// The set must offer each of them once.
///
/// FALSIFICATION (measured 2026-08-14, with both `dedupe_within_sets` calls
/// removed — from `scan_stories` and from `StorySource::scan`):
///
/// ```text
/// thread 'disk_set_rows::a_build_carried_by_two_volumes_is_one_row' panicked at
/// crates/app/tests/suites/disk_set_rows.rs:372:13:
/// assertion `left == right` failed: ZCODE-107-870430-2871 is offered 2 times by one release: ["Infocom Compilation 1 (19xx)(-).st::STATION.T", "Infocom Compilation 8 (19xx)(-).st::STATION/STORY.DAT"]
///   left: 2
///  right: 1
/// ```
///
/// Stationfall rather than Trinity only because the map is ordered by IFID and
/// `ZCODE-107-…` sorts first; `trinity_is_offered_once_by_the_atari_st_shelf`
/// failed in the same run with *"Trinity r11/860509 is offered 2 times"*.
#[test]
fn a_build_carried_by_two_volumes_is_one_row() {
    let mut ran = 0;
    for set in SETS {
        let path = stories_dir().join(set.member);
        if !path.is_file() {
            continue;
        }
        let base = data_base("dedupe");
        ran += 1;
        let members = app::disk_set::members(&path).expect("a set");
        let rows = app::picker::StorySource::of(&path, &base)
            .expect("a set worth offering")
            .scan(&base);

        // What the mounts hold, before anything folds: the `raw` column.
        let on_disk = ifids_on(&members);
        assert_eq!(
            on_disk.len(),
            set.raw,
            "{}: the media carry {} stories, not {}",
            set.member,
            on_disk.len(),
            set.raw,
        );

        // Every build is offered exactly once.
        let mut by_ifid: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for r in &rows {
            by_ifid.entry(&r.meta.ifid).or_default().push(format!(
                "{}::{}",
                r.filename,
                r.meta.disk_entry.clone().unwrap_or_else(|| "-".into())
            ));
        }
        for (ifid, where_) in &by_ifid {
            assert_eq!(
                where_.len(),
                1,
                "{ifid} is offered {} times by one release: {where_:?}",
                where_.len(),
            );
        }
        assert_eq!(by_ifid.len(), set.games, "{}: distinct builds", set.member);

        // …and nothing was lost: every build on every volume is still offered.
        for (vol, name, ifid) in &on_disk {
            assert!(
                by_ifid.contains_key(ifid.as_str()),
                "{}: {name} ({ifid}) off {vol:?} left the release entirely",
                set.member,
            );
        }
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no release was deduped");
}

/// The named case, spelled out: `ZCODE-11-860509-FAAE` on two Atari ST volumes,
/// stored flat on one and in a directory on the other.
#[test]
fn trinity_is_offered_once_by_the_atari_st_shelf() {
    let dir = stories_dir();
    let (c5, c8) = (
        dir.join("Infocom Compilation 5 (19xx)(-).st"),
        dir.join("Infocom Compilation 8 (19xx)(-).st"),
    );
    if !c5.is_file() || !c8.is_file() {
        return;
    }
    const TRINITY: &str = "ZCODE-11-860509-FAAE";
    // The premise: both volumes really do carry that one build.
    let on_disk = ifids_on(&[c5.clone(), c8.clone()]);
    let carriers: Vec<&String> =
        on_disk.iter().filter(|(_, _, i)| i == TRINITY).map(|(_, n, _)| n).collect();
    assert_eq!(carriers.len(), 2, "the premise moved: {TRINITY} is on {carriers:?}");
    assert!(carriers.iter().any(|n| n.contains('/')), "one copy is stored in a directory");

    let base = data_base("trinity");
    let rows =
        app::picker::StorySource::of(&c5, &base).expect("the ST shelf is a set").scan(&base);
    let offered: Vec<&app::picker::StoryEntry> =
        rows.iter().filter(|r| r.meta.ifid == TRINITY).collect();
    assert_eq!(offered.len(), 1, "Trinity r11/860509 is offered {} times", offered.len());
    // The earlier volume keeps it, deterministically — disk 5, not disk 8.
    assert_eq!(offered[0].path, c5, "the lowest disk number keeps the build");
    let _ = std::fs::remove_dir_all(&base);
}

/// **Guard: dedupe removes duplicates and nothing else.** Zork Zero is the
/// sharpest control in the corpus — r296 on a Macintosh HFS volume, r366 on an
/// Amiga floppy and r393 on the DOS media — three genuinely different builds of
/// one game that must stay three rows however the list is folded. And r393
/// itself is reached through three separate sets plus a loose `.z6`; those are
/// four pieces of media the player keeps on purpose, and no two of them are
/// volumes of one release, so all four survive.
#[test]
fn different_builds_of_one_game_are_never_folded_together() {
    let dir = stories_dir();
    if !dir.is_dir() {
        return;
    }
    let base = data_base("zork0");
    let rows = app::picker::scan_stories(&dir, &base);
    let zz: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.title.contains("Zork Zero"))
        .map(|r| r.meta.ifid.as_str())
        .collect();
    if zz.is_empty() {
        let _ = std::fs::remove_dir_all(&base);
        return; // no Zork Zero media in this corpus
    }
    for (release, ifid) in [
        (296, "ZCODE-296-881019-8C61"),
        (366, "ZCODE-366-890323-C5CD"),
        (393, "ZCODE-393-890714-791C"),
    ] {
        if !dir.join("Zork Zero Disk.image").is_file() && release == 296 {
            continue;
        }
        if !dir.join("Zork Zero - The Revenge of Megaboz.adf").is_file() && release == 366 {
            continue;
        }
        if !dir.join("floppy5.ima").is_file() && release == 393 {
            continue;
        }
        assert!(zz.contains(ifid), "Zork Zero r{release} ({ifid}) was folded away: {zz:?}");
    }

    // The same build off unrelated media stays one row per medium.
    let r393: Vec<&PathBuf> = rows
        .iter()
        .filter(|r| r.meta.ifid == "ZCODE-393-890714-791C")
        .map(|r| &r.path)
        .collect();
    if r393.len() > 1 {
        for a in &r393 {
            for b in &r393 {
                if a == b {
                    continue;
                }
                let same_set =
                    app::disk_set::members(a).is_some_and(|m| m.contains(b));
                assert!(!same_set, "two volumes of one set both offer r393: {a:?} {b:?}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// **Guard: a set's games keep their own saves** (SQ-0850's promise, across
/// volumes now). Every game the release offers resolves to its own directory,
/// and the same build reached through two disks of one set resolves to **one**
/// directory — which is what the dedupe leans on and what makes it safe.
#[test]
fn a_releases_games_keep_their_own_saves() {
    let mut ran = 0;
    for set in SETS {
        let path = stories_dir().join(set.member);
        if !path.is_file() {
            continue;
        }
        let base = data_base("saves");
        ran += 1;
        let members = app::disk_set::members(&path).expect("a set");
        let rows = app::picker::StorySource::of(&path, &base).expect("a set").scan(&base);
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for r in &rows {
            let dir = r.game_dir(&base);
            assert!(dirs.insert(dir.clone()), "{}: two games share {}", set.member, dir.display());
            // The launch arrives at the same key from (path, selector) alone.
            assert_eq!(
                app::storage::story_key_at_from(&r.path, r.meta.disk_entry.as_deref()),
                r.story_key(),
                "{}: {} keys differently at launch",
                set.member,
                r.title,
            );
        }
        assert_eq!(dirs.len(), rows.len(), "{}: {} games, {} directories", set.member, rows.len(), dirs.len());

        // The property the dedupe rests on: one build, one directory, whichever
        // volume of the set it is reached through.
        let mut key_of: BTreeMap<String, String> = BTreeMap::new();
        for (vol, name, ifid) in ifids_on(&members) {
            let key = app::storage::story_key_at_from(&vol, Some(&name));
            if let Some(prev) = key_of.insert(ifid.clone(), key.clone()) {
                assert_eq!(
                    prev, key,
                    "{}: {ifid} resolves to two save directories across the release",
                    set.member,
                );
            }
        }
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no release was keyed");
}

// ── The directory scan is unaffected outside a set ───────────────────────────

/// **Guard: single images and non-set disks are unaffected.** Every row a
/// single-title floppy contributed before still contributes, unchanged, and its
/// save key has not moved.
#[test]
fn a_single_image_is_exactly_what_it_was() {
    let mut ran = 0;
    for name in SINGLE_TITLE {
        let path = stories_dir().join(name);
        if !path.is_file() {
            continue;
        }
        let base = data_base("single");
        let rows = app::picker::resolve_entries(&path, &base);
        let Some(old) = app::picker::resolve_entry(&path, &base) else {
            let _ = std::fs::remove_dir_all(&base);
            continue; // mountable but carrying nothing launchable
        };
        ran += 1;
        assert_eq!(rows.len(), 1, "{name}: a single-title floppy is one row");
        assert_eq!(rows[0], old, "{name}: the row must be the old answer byte for byte");
        assert_eq!(rows[0].meta.disk_entry, None, "{name}: nothing to choose");
        assert_eq!(rows[0].story_key(), app::storage::story_key_at(&path), "{name}: key moved");

        // …and it survives the whole-directory scan as exactly one row.
        let listed = app::picker::scan_stories(&stories_dir(), &base);
        assert_eq!(
            listed.iter().filter(|e| e.path == path).count(),
            1,
            "{name}: the scan folded or duplicated a lone floppy",
        );
        let _ = std::fs::remove_dir_all(&base);
    }
    assert!(ran > 0 || !any_media_present(), "media are present but no single image ran");
}

/// A loose `.z*` story file is untouched by any of this: still one row, still
/// listed, never folded into anything.
#[test]
fn loose_story_files_are_untouched() {
    let dir = stories_dir();
    if !dir.is_dir() {
        return;
    }
    let base = data_base("loose");
    let listed = app::picker::scan_stories(&dir, &base);
    let mut ran = 0;
    for name in ["zork1.z5", "trinity-r12-s860926.z4", "stationfall-r107-s870430.z3"] {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        ran += 1;
        assert_eq!(
            listed.iter().filter(|e| e.path == path).count(),
            1,
            "{name}: a loose story file must be listed exactly once",
        );
        assert!(app::disk_set::members(&path).is_none(), "{name}: grouped into a set");
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !dir.join("zork1.z5").exists());
}

// ── One rule, one place (SQ-0874) ────────────────────────────────────────────

/// **The two front-ends agree about every set in the corpus**, because they are
/// asking the same function.
///
/// The rule lived in `app` until `zvm-cli` needed it, and a CLI cannot depend on
/// `app` — so the choice was to move it down to `cli-host` (which both already
/// link) or to copy it sideways. A copy is how two front-ends end up with two
/// ideas of what a release is, and the disagreement is invisible until a game
/// goes missing from one of them.
///
/// This walks every disk image in `stories/` and asserts the answers are
/// identical, member for member and in the same order — the browser's shelves
/// and the CLI's mount are the same sets.
#[test]
fn the_browser_and_the_cli_group_the_corpus_identically() {
    let dir = stories_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut ran = 0;
    let mut sets = 0;
    for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
        ran += 1;
        let via_app = app::disk_set::members(&path);
        let via_cli = cli_host::disk_set::members(&path);
        assert_eq!(via_app, via_cli, "{}: the two front-ends disagree", path.display());
        if via_app.is_some() {
            sets += 1;
        }
    }
    assert!(ran > 0 || !any_media_present(), "media are present but nothing was compared");
    // …and the comparison had real sets in it, rather than agreeing on `None`
    // for everything: the Apple II presses and the Commodore *Trinity* are here.
    assert!(sets > 0 || !any_media_present(), "no set in the corpus was compared at all");
}

/// …and "the same function" is a fact about the source, not a hope.
///
/// `app::disk_set` is a re-export, so the equality above could never fail — which
/// is exactly the point, and this is what stops someone restoring the second copy
/// that would make it able to. Exactly one file in the workspace implements the
/// rule; the census is over `is_index_run`, the clause that decides whether a
/// digit run is a disk number.
///
/// Needs no fixture, so it never skips: CI has no `stories/` and this is the half
/// of the guard that still runs there.
#[test]
fn exactly_one_file_in_the_workspace_implements_the_rule() {
    // Spelt in two halves so this file is not itself a hit: the census walks
    // every `.rs` in the workspace, including this one.
    let needle = concat!("fn ", "is_index_run");
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates");
    let mut carriers: Vec<PathBuf> = Vec::new();
    let mut stack = vec![crates.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("target")) {
                    continue;
                }
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && std::fs::read_to_string(&p).is_ok_and(|s| s.contains(needle))
            {
                carriers.push(p);
            }
        }
    }
    assert_eq!(carriers.len(), 1, "the multi-disk rule is implemented in {carriers:?}");
    assert!(
        carriers[0].ends_with("cli-host/src/disk_set.rs"),
        "the rule must live where both front-ends reach it, not in {:?}",
        carriers[0]
    );
}

/// The whole scan, counted: folding a set never removes a *game*, only a second
/// copy of one. Every build on every mountable image in the directory is
/// somewhere in the list.
#[test]
fn the_scan_loses_no_game_anywhere() {
    let dir = stories_dir();
    if !dir.is_dir() {
        return;
    }
    let base = data_base("whole");
    let listed = app::picker::scan_stories(&dir, &base);
    let offered: BTreeSet<&str> = listed.iter().map(|e| e.meta.ifid.as_str()).collect();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let images: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    let mut ran = 0;
    for (vol, name, ifid) in ifids_on(&images) {
        ran += 1;
        assert!(
            offered.contains(ifid.as_str()),
            "{name} ({ifid}) is on {vol:?} and nowhere in the browser",
        );
    }
    let _ = std::fs::remove_dir_all(&base);
    assert!(ran > 0 || !any_media_present(), "media are present but nothing was counted");
}
