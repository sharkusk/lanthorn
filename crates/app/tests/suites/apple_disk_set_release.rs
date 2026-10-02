//! **Naming whichever floppy is on top opens the game** (SQ-0864).
//!
//! The Apple II 5.25-inch presses of *Shogun* and *Zork Zero* are five and four
//! volumes, and one story paged across all of them. `blorb`'s own suite pins the
//! reassembly; this file pins the thing a person actually does, through the
//! seams the app actually uses — [`app::disk_set`] to say which files are one
//! release, [`app::hints`] to mount across them, and [`app::picker`] to decide
//! what the browser shows.
//!
//! The distinction matters because the two halves can each be right and still
//! not meet. `blorb` is handed bytes and never learns what a directory is; `app`
//! knows the directory and must not learn what a `.dsk` is. If the join is
//! wrong, every test in `blorb` still passes and no game opens.
//!
//! `stories/` is gitignored, so every case skips vacuously without the media and
//! CI has none of it at all — hence `ran > 0 || !present()` rather than a bare
//! premise assertion.

use std::path::{Path, PathBuf};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The presses: every volume, and the build the whole set carries.
///
/// *Journey* joined them in SQ-0863, when its 5.25-inch set arrived. It is the
/// same shape as the other two and it settles something they could not: the
/// `Journey.2mg` in the corpus declares five segments and carries four, and
/// until this set there was no way to tell a short IMAGE from a release lanthorn
/// could not read. Release 77 comes off all five floppies, so the `.2mg` is a
/// short pressing and the reader was right to refuse it.
const SETS: &[(&str, &[&str], u16, &str, usize)] = &[
    (
        "Shogun",
        &[
            "shogun_s1.dsk",
            "shogun_s2.dsk",
            "shogun_s3.dsk",
            "shogun_s4.dsk",
            "shogun_s5.dsk",
        ],
        311,
        "890510",
        344_224,
    ),
    (
        "Zork Zero",
        &["zork_zero_1.dsk", "zork_zero_2.dsk", "zork_zero_3.dsk", "zork_zero_4.dsk"],
        383,
        "890602",
        299_392,
    ),
    (
        "Journey",
        &[
            "journey_s1.dsk",
            "journey_s2.dsk",
            "journey_s3.dsk",
            "journey_s4.dsk",
            "journey_s5.dsk",
        ],
        77,
        "890616",
        282_176,
    ),
];

/// Is any volume of any set here?
fn any_present() -> bool {
    SETS.iter().flat_map(|(_, v, ..)| v.iter()).any(|f| stories_dir().join(f).exists())
}

/// Every volume of `set`, or `None` unless all of them are present — a partial
/// set is a different (and legitimately story-less) thing, not a failure.
fn complete(volumes: &[&str]) -> Option<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = volumes.iter().map(|v| stories_dir().join(v)).collect();
    paths.iter().all(|p| p.exists()).then_some(paths)
}

/// **The headline, from the app's side.** Whichever volume a person names, the
/// whole game opens — the same release, byte for byte, off all five floppies.
///
/// This is not a restatement of `blorb`'s suite: nothing here hands `blorb` a
/// set. It hands `app::hints` a single path, and the sibling discovery, the
/// reads and the mount are the app's own chain doing what it does at launch.
///
/// FALSIFICATION: make `hints::mount_disk`'s companion closure return an empty
/// list — the change that leaves every line of `blorb` intact and breaks only
/// the join — and this fails on the first volume with the originally reported
/// symptom, verbatim:
///
/// ```text
/// shogun_s1.dsk: Shogun should open off this volume: no story file on the
/// disk image …/stories/shogun_s1.dsk (4 files on SHOGUN.1; is this the boot
/// disk?)
/// ```
#[test]
fn every_volume_of_a_five_and_a_quarter_inch_press_opens_the_whole_game() {
    let mut ran = 0;
    for (title, volumes, release, serial, length) in SETS {
        let Some(paths) = complete(volumes) else {
            eprintln!("SKIP: {title}'s press is not complete in stories/");
            continue;
        };
        ran += 1;
        let mut builds: Vec<Vec<u8>> = Vec::new();
        for path in &paths {
            let who = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let (loaded, image) = app::hints::load_mounted_story(path)
                .unwrap_or_else(|e| panic!("{who}: {title} should open off this volume: {e}"));
            assert_eq!(
                image,
                Some(app::hints::DiskImage::ProDos),
                "{who}: a 5.25-inch press is a ProDOS volume in the drive's sector order",
            );
            let bytes = loaded.bytes().to_vec();
            assert_eq!(bytes[0], 6, "{who}: Version 6");
            assert_eq!(bytes.len(), *length, "{who}: the story's declared length");
            assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{who}");
            assert_eq!(&bytes[0x12..0x18], serial.as_bytes(), "{who}");
            builds.push(bytes);
        }
        // Not merely "each volume yields A story" — they yield THE SAME one.
        assert!(
            builds.windows(2).all(|w| w[0] == w[1]),
            "{title}: the volumes disagree about what game this is",
        );
    }
    assert!(ran > 0 || !any_present(), "media are present but no set was complete");
    if ran == 0 {
        eprintln!("SKIP: no 5.25-inch media present");
    }
}

/// **The browser shows two games, not nine disks.**
///
/// Five volumes each reporting the same reassembled build is five rows before
/// `dedupe_within_sets` and one after, and the lowest disk number keeps it
/// (SQ-0844). That fold is the whole reason a set model is worth having here:
/// without it a shelf of nine floppies reads as nine copies of two games.
#[test]
fn a_press_is_one_row_in_the_browser_and_the_first_volume_keeps_it() {
    let Some(_) = complete(SETS[0].1) else {
        eprintln!("SKIP: Shogun's press is not complete in stories/");
        assert!(!any_present() || complete(SETS[1].1).is_none());
        return;
    };
    let dir = stories_dir();
    let rows = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&dir));
    let dsk: Vec<String> = rows
        .iter()
        .filter(|e| {
            e.path.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("dsk"))
                == Some(true)
        })
        .map(|e| e.path.file_name().unwrap_or_default().to_string_lossy().into_owned())
        .collect();
    // Zork Zero's press may legitimately be absent; Shogun's is here.
    assert!(dsk.contains(&"shogun_s1.dsk".to_string()), "{dsk:?}");
    for absent in ["shogun_s2.dsk", "shogun_s3.dsk", "shogun_s4.dsk", "shogun_s5.dsk"] {
        assert!(!dsk.contains(&absent.to_string()), "{absent} is a second row for one game");
    }
    if SETS.iter().all(|(_, v, ..)| complete(v).is_some()) {
        // By title, which is the scan's own order — three presses, three rows,
        // and fourteen floppies folded into them.
        //
        // …and one row that is not a press at all (SQ-0868). *Planetfall*'s
        // retail disk is a RAW self-booting `.dsk` — one whole game on one
        // floppy, no filesystem, a second format wearing this spelling — so it
        // is a fourth row and must never be a fifteenth floppy folded into
        // somebody else's set. Its stem carries a digit run (`r29`), which is
        // exactly what would make a careless set rule swallow it;
        // `app::disk_set`'s unit tests pin the refusal and this pins the
        // consequence a person would actually see.
        assert_eq!(
            dsk,
            [
                "journey_s1.dsk",
                "Planetfall r29 (clean copy from retail disk).dsk",
                "shogun_s1.dsk",
                "zork_zero_1.dsk",
            ],
            "three presses and one lone raw disk, four rows"
        );
    }
}

/// **`app::disk_set` needed no change to see these**, which is the seam being in
/// the right place rather than a coincidence.
///
/// The rule reads its extension census off `blorb::medium`'s table, so the day
/// the ProDOS row claimed `.dsk` the nine files became two sets. Pinned over the
/// real directory (not synthetic names) because the real directory is where the
/// two presses sit beside ten `.2mg`s, nine `.st`s and a shelf of `.adf`s that
/// must not be dragged in with them.
#[test]
fn the_real_directory_groups_the_nine_volumes_into_exactly_two_releases() {
    let Some(shogun) = complete(SETS[0].1) else {
        eprintln!("SKIP: Shogun's press is not complete in stories/");
        assert!(!any_present() || complete(SETS[1].1).is_none());
        return;
    };
    for volume in &shogun {
        let members = app::disk_set::members(volume)
            .unwrap_or_else(|| panic!("{}: is in no set", volume.display()));
        assert_eq!(members, shogun, "{}: the set is the five Shogun floppies", volume.display());
    }
    // A ProDOS volume that is NOT one of a numbered run is left alone, so the
    // widening cannot reach across unrelated Apple II media.
    let standalone = stories_dir().join("Beyond Zork (1988)(Infocom).2mg");
    if standalone.exists() {
        assert_eq!(app::disk_set::members(&standalone), None, "a lone volume is no set");
    }
}

/// **A volume of a set, opened out of its own directory, is still refused
/// honestly.** Copy one floppy somewhere on its own and there is nothing to
/// assemble — so the answer is the "is this the boot disk?" message, never four
/// fifths of a game.
///
/// This is the case that says the set is *reached through the filesystem* and
/// not smuggled in some other way: same bytes, no siblings, no story.
#[test]
fn one_volume_alone_in_a_directory_yields_no_story() {
    let source = stories_dir().join("shogun_s3.dsk");
    let Ok(raw) = std::fs::read(&source) else {
        eprintln!("SKIP: gitignored medium missing at {}", source.display());
        assert!(!any_present());
        return;
    };
    let dir = std::env::temp_dir().join(format!("lanthorn-sq0864-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let lone = dir.join("shogun_s3.dsk");
    std::fs::write(&lone, &raw).expect("write the lone volume");

    let mounted = app::hints::mounted_stories(&lone);
    let err = app::hints::load_mounted_story(&lone).err().map(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&dir);

    assert!(mounted.is_none(), "a lone volume offers no story: {mounted:?}");
    let err = err.expect("a lone volume must not open a game");
    assert!(err.contains("no story file on the disk image"), "{err}");
    // The message names the ProDOS volume, because the mount really did succeed
    // — this is a disk we read, not a disk we could not.
    assert!(err.contains("SHOGUN.3"), "{err}");
}

/// The other half of the guard above: it is not that `.dsk` is special, it is
/// that a volume with a game on it never consults its release at all. A
/// compilation `.2mg` sitting in the same directory as the two presses answers
/// for itself, with the same games it always did.
#[test]
fn a_volume_that_carries_games_is_untouched_by_the_set_path() {
    let path = stories_dir().join(
        "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg",
    );
    let Some((image, stories)) = app::hints::mounted_stories(&path) else {
        eprintln!("SKIP: gitignored medium missing at {}", path.display());
        assert!(!path.exists());
        return;
    };
    assert_eq!(image, app::hints::DiskImage::ProDos);
    let names: Vec<&str> = stories.iter().map(|(s, _)| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["ZORK.III", "ZORK.II", "ZORK.I", "HITCHHIKER", "BEYOND.ZORK"],
        "its six sibling volumes contributed nothing, which is correct: it is a shelf",
    );
}

/// A `.dsk` that is not a ProDOS volume is refused rather than misread — the
/// generous extension pre-filter is safe only because content decides.
///
/// Most `.dsk` files in the world are Apple II DOS 3.3 disks, which this crate
/// does not read; one is written here as a file of the exact 5.25-inch size so
/// the size test cannot be what saves us.
#[test]
fn a_dsk_that_is_not_a_prodos_volume_is_not_listed() {
    let dir = std::env::temp_dir().join(format!("lanthorn-sq0864-junk-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // 143,360 bytes — the right geometry, no volume directory anywhere in it,
    // and byte `$00` is `$E5` (the classic format filler) rather than anything
    // a Z-machine version could be. That last part was not fussiness: a file the
    // mount declined fell through to being read as a plain story, so a fixture
    // whose first byte was 6, 7 or 8 got listed as a story and this case passed
    // for the wrong reason. SQ-0889 closed that fallthrough — a header, not a
    // version byte, is what claims a file for the Z-machine now — so the `$E5`
    // is belt and braces rather than the thing holding this case up.
    let noise: Vec<u8> = (0..143_360usize).map(|i| (i * 31 + 7) as u8).collect();
    let noise: Vec<u8> = std::iter::once(0xE5).chain(noise.into_iter().skip(1)).collect();
    std::fs::write(dir.join("homebrew1.dsk"), &noise).unwrap();
    std::fs::write(dir.join("homebrew2.dsk"), &noise).unwrap();
    // A real story beside them, so an empty list cannot pass by accident — and
    // a real one it has to be, with the ZMSD §1.1 memory map it claims: dynamic
    // memory below `$0e`, the writable object and global tables inside it, the
    // dictionary above it (SQ-0889).
    let mut story = vec![0u8; 4096];
    story[0] = 3;
    story[0x04..0x06].copy_from_slice(&0x0200u16.to_be_bytes()); // high memory base
    story[0x08..0x0a].copy_from_slice(&0x0400u16.to_be_bytes()); // dictionary
    story[0x0a..0x0c].copy_from_slice(&0x0080u16.to_be_bytes()); // object table
    story[0x0c..0x0e].copy_from_slice(&0x0100u16.to_be_bytes()); // globals
    story[0x0e..0x10].copy_from_slice(&0x0200u16.to_be_bytes()); // static memory base
    story[0x12..0x18].copy_from_slice(b"840726");
    story[0x1a..0x1c].copy_from_slice(&((4096u16 / 2).to_be_bytes()));
    std::fs::write(dir.join("game.z3"), &story).unwrap();

    let rows = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&dir));
    let names: Vec<String> = rows.iter().map(|e| e.filename.clone()).collect();
    // They ARE grouped as a set — the rule is about names and does not open
    // anything — and they still list nothing, because the mount refuses them.
    let grouped = app::disk_set::members(&dir.join("homebrew1.dsk")).map(|m| m.len());
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(grouped, Some(2), "the naming rule groups them; the mount is what declines");
    assert_eq!(names, ["game.z3"], "a .dsk that is not a ProDOS volume is never listed: {names:?}");
}

/// Where this belongs in the corpus, stated once: the 5.25-inch press is a
/// FIFTH Shogun and a FOURTH Zork Zero, and no two of those media carry the same
/// build. `real_media_releases.rs` pins each of them; this asserts the spread,
/// because it is the project's "a disk image is a different release" rule at
/// full stretch and it is now cheap to lose.
#[test]
fn the_five_and_a_quarter_inch_press_is_a_build_no_other_medium_carries() {
    let media: &[(&str, u16)] = &[
        ("James Clavell's Shogun.adf", 295),
        ("shogun-r322-s890706.z6", 322),
        ("shogun_s1.dsk", 311),
    ];
    let mut seen: Vec<u16> = Vec::new();
    for (file, release) in media {
        let path = stories_dir().join(file);
        let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            continue;
        };
        let bytes = loaded.bytes();
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{file}");
        seen.push(*release);
    }
    let unique = {
        let mut s = seen.clone();
        s.sort_unstable();
        s.dedup();
        s.len()
    };
    assert_eq!(unique, seen.len(), "three Shoguns, three releases: {seen:?}");
    assert!(seen.len() == 3 || !stories_dir().join("shogun_s1.dsk").exists());
}

/// Every path above went through the app. This one checks the one thing the app
/// must NOT have: knowledge of the format. Nothing in `app` names `.dsk`, so the
/// spelling can only have arrived from `blorb::medium`'s table.
#[test]
fn the_app_learned_the_spelling_from_the_table_and_not_from_itself() {
    assert!(blorb::medium::image_extensions().any(|e| e == "dsk"));
    // The scan's own pre-filter is crate-private and pinned in `picker`'s unit
    // tests over exactly this census; what is checked here is the census, which
    // is the thing `app` would have had to hard-code to get this wrong.
    // …and the row that claims it is ProDOS, so the machine comes with it.
    assert_eq!(app::hints::DiskImage::ProDos.label(), "ProDOS");
    assert_eq!(
        app::hints::DiskImage::ProDos.interpreter_number(),
        Some(10),
        "ZMSD §11.1.3: 10 = Apple IIgs — a 5.25-inch press is Apple II media like the 3.5-inch one",
    );
}
