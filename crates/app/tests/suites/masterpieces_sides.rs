//! SQ-0876: the two halves of a hybrid disc, told apart end to end.
//!
//! *Classic Text Adventure Masterpieces of Infocom* is one CD whose Macintosh
//! partition carries Infocom's **DOS** builds as well as its Macintosh ones — 50
//! of the former and 33 of the latter, 83 launchable stories on one platter. Two
//! questions used to be answered per FORMAT and had to become per STORY:
//!
//! * **which machine** — the row said `HFS` for all 83, so every PC build
//!   advertised interpreter number 3, the Macintosh (ZMSD §11.1.3), which for a
//!   Version 6 story is the byte the standard singles out as consequential;
//! * **which artwork** — one archive answered for the whole volume, so all six
//!   graphical games resolved to `MAC/ZORK ZERO/CPIC.DATA`, which wins the
//!   volume-wide tiebreak on picture count. Opening Journey drew Zork Zero's
//!   plates, and looked like artwork the whole time.
//!
//! `blorb` owns the rule (`hfs::HfsEntry::is_from_dos`, over the Finder creator
//! Apple's PC Exchange stamps on an import) and pins it against the disc. What
//! this suite pins is the half `app` is responsible for: that the answer reaches
//! the browser row, the interpreter profile and the resolved artwork.
//!
//! The disc lives outside the repo — `masterpieces/` is not in git, and it is
//! 354 MB — so every case skips vacuously when it is absent. CI has none of it.

use std::path::{Path, PathBuf};

use app::interpreter::InterpreterProfile;

fn disc() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../masterpieces/Classic Text Adventure Masterpieces of Infocom (USA).bin");
    p.is_file().then_some(p)
}

/// Every browser row off the disc reports the machine ITS OWN story came off.
///
/// The `(HFS)`/`(DOS)` the picker shows is `StoryMeta::disk_image`, so this is
/// the badge, one layer below the formatting.
#[test]
fn every_row_reports_the_machine_its_own_story_came_off() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    let base = std::env::temp_dir().join("bm-masterpieces-sides");
    // 66, not 83: seventeen DOS builds are on the disc twice and fold
    // (SQ-0878, pinned in full below).
    let rows = app::picker::resolve_entries(&disc, &app::data_roots::DataRoots::single(&base));
    assert_eq!(rows.len(), 66, "one row per launchable build, per machine");

    let mut mac = 0;
    let mut dos = 0;
    for row in &rows {
        let entry = row.meta.disk_entry.as_deref().expect("a compilation row names its story");
        let want = if entry.starts_with("PC/") {
            dos += 1;
            blorb::medium::DiskImage::Fat12Dos
        } else {
            mac += 1;
            blorb::medium::DiskImage::Hfs
        };
        assert_eq!(row.meta.disk_image, Some(want), "{entry}");
    }
    assert_eq!((mac, dos), (33, 33), "the disc's two halves, each build once");
}

/// **One build per machine, once** (SQ-0878) — and neither of the two traps
/// this disc sets is fallen into.
///
/// Seventeen DOS builds are on the disc twice, because the 1996 reissue put
/// each game in its own folder and left the 1991 shared `PC/DATA/` directory
/// in place. Those fold. What must NOT fold:
///
/// * *Cutthroats* is **byte-identical** across the two machines, all 112,640 of
///   them, so comparing bytes would merge a Macintosh row with a DOS one.
/// * Mac and DOS ship the **same build** of many games — Zork I is r88/840726
///   on both sides — so comparing the IFID alone would do the same.
/// * *Beyond Zork*'s only DOS copy is in `PC/DATA/`: the one game the
///   reorganisation missed. Folding on the folder rather than the build would
///   lose it outright.
#[test]
fn one_build_per_machine_survives_and_the_cross_machine_pairs_do_not_fold() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    let base = std::env::temp_dir().join("bm-masterpieces-dedupe");
    let rows = app::picker::resolve_entries(&disc, &app::data_roots::DataRoots::single(&base));
    assert_eq!(rows.len(), 66, "83 stories, seventeen of them the same build twice");

    let entry = |row: &app::picker::StoryEntry| row.meta.disk_entry.clone().unwrap_or_default();
    let names: Vec<String> = rows.iter().map(entry).collect();

    // The vestigial 1991 directory is gone…
    assert!(
        !names.iter().any(|n| n == "PC/DATA/ZORK1.DAT"),
        "the shared copy folds into the per-game one the disc's README runs"
    );
    assert!(names.iter().any(|n| n == "PC/ZORK1/DATA/ZORK1.DAT"), "…and that one survives");
    // …except where it is the only copy there is.
    assert!(
        names.iter().any(|n| n == "PC/DATA/BEYONDZO.DAT"),
        "Beyond Zork has no per-game folder; its only DOS copy must survive"
    );

    // Both machines' pressings survive, including the byte-identical pair.
    for (mac, dos) in [
        ("MAC/CUTTHROATS", "PC/CUTTHROA/CUTTHROA.DAT"),
        ("MAC/ZORK I", "PC/ZORK1/DATA/ZORK1.DAT"),
    ] {
        assert!(names.iter().any(|n| n == mac), "{mac}");
        assert!(names.iter().any(|n| n == dos), "{dos}");
    }
    // …and no row is a duplicate of another for the same machine.
    let mut keys: Vec<(Option<&'static str>, String)> = rows
        .iter()
        .map(|r| (r.meta.disk_image.map(|d| d.label()), r.meta.ifid.clone()))
        .collect();
    let before = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), before, "a build is offered once per machine");
}

/// The machine each half resolves to, through the profile the boot actually
/// uses.
///
/// The DOS half answers [`InterpreterProfile::IbmPc`], which advertises **no**
/// number of its own — deliberately, since the IBM PC's honest answer is
/// version-dependent (Frotz's rule: 6 for Version 6, 1 otherwise) and no single
/// constant expresses it. So the fix is that a PC build stops CLAIMING the
/// Macintosh, not that it starts claiming 6.
#[test]
fn each_half_resolves_to_its_own_machine() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    let mac = InterpreterProfile::resolve(&disc, None, None, Some(blorb::medium::DiskImage::Hfs));
    assert_eq!(mac, InterpreterProfile::Macintosh);
    assert_eq!(mac.interpreter_number(), Some(3), "ZMSD §11.1.3: 3 = Macintosh");

    let pc =
        InterpreterProfile::resolve(&disc, None, None, Some(blorb::medium::DiskImage::Fat12Dos));
    assert_eq!(pc, InterpreterProfile::IbmPc);
    assert_eq!(pc.interpreter_number(), None, "the IBM PC leaves the version rule in force");

    // An explicit number still outranks the medium, on either half.
    assert_eq!(
        InterpreterProfile::resolve(&disc, Some(4), None, Some(blorb::medium::DiskImage::Hfs)),
        InterpreterProfile::Amiga,
        "an explicit interpreter number wins, as it always has"
    );
}

/// Each graphical game draws with the archive in its own folder — and a text
/// game with no artwork gets none rather than a stranger's.
///
/// `.MG1` for the DOS Arthur and Journey and `.EG1` for the DOS Zork Zero is the
/// existing colour-first rendition rule working, not a new one: no `.MG1` was
/// pressed for Zork Zero, so EGA is the best colour rendition on the disc.
#[test]
fn each_graphical_game_draws_with_its_own_folders_archive() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    for (entry, want) in [
        ("MAC/ARTHUR FOLDER/STORY.DATA", Some("MAC/ARTHUR FOLDER/CPIC.DATA")),
        ("MAC/JOURNEY FOLDER/STORY.DATA", Some("MAC/JOURNEY FOLDER/CPIC.DATA")),
        ("MAC/ZORK ZERO/STORY.DATA", Some("MAC/ZORK ZERO/CPIC.DATA")),
        ("PC/ARTHUR/ARTHUR.ZIP", Some("PC/ARTHUR/ARTHUR.MG1")),
        ("PC/JOURNEY/JOURNEY.ZIP", Some("PC/JOURNEY/JOURNEY.MG1")),
        ("PC/ZORK0/ZORK0.ZIP", Some("PC/ZORK0/ZORK0.EG1")),
        ("MAC/ZORK I", None),
        ("PC/ZORK1/DATA/ZORK1.DAT", None),
    ] {
        let art = app::graphics::release_art(&disc, Some(entry));
        assert_eq!(art.map(|a| a.name).as_deref(), want, "the artwork paired with {entry}");
    }
}

/// The options panel offers ONE game's archives, not the whole platter's.
///
/// The medium's guarantee is "it shipped in the box the story was mounted out
/// of", and that was enough while a box held one game. This disc's box holds
/// six, so the unfiltered list offered all sixteen archives on it — Macintosh
/// and DOS renditions of three other games — for whichever story you opened.
#[test]
fn the_options_panel_offers_only_this_storys_own_archives() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    // The whole platter, which is what a person used to be shown for any of the
    // 83 stories: six Macintosh archives (three games, colour and monochrome
    // each) and ten DOS ones (Arthur and Journey at four renditions, Zork Zero
    // at two) — less the two `.EG2` continuations, which are the back halves of
    // their `.EG1` rows rather than rows of their own.
    let unfiltered = app::launch_options::discover_art_candidates(&disc, None, None);
    assert_eq!(unfiltered.len(), 14, "every archive on the disc, EGA sets counted once");

    for (entry, want) in [
        ("MAC/JOURNEY FOLDER/STORY.DATA", vec!["CPIC.DATA", "PIC.DATA"]),
        ("PC/ZORK0/ZORK0.ZIP", vec!["ZORK0.CG1", "ZORK0.EG1"]),
        // `.EG2` is the back half of `.EG1`, not a choice of its own (SQ-0881).
        ("PC/ARTHUR/ARTHUR.ZIP", vec!["ARTHUR.CG1", "ARTHUR.EG1", "ARTHUR.MG1"]),
        // A text game shipped beside no artwork is offered none.
        ("MAC/ZORK I", vec![]),
    ] {
        let mut got: Vec<String> = app::launch_options::discover_art_candidates(&disc, Some(entry), None)
            .into_iter()
            .map(|c| c.filename.rsplit('/').next().unwrap_or_default().to_string())
            .collect();
        got.sort();
        assert_eq!(got, want, "the archives offered for {entry}");
    }
}

/// The launch dialog's "what this will draw with" row agrees with the boot, per
/// story.
///
/// It has to be asked through `on_disk_entry`, because which game on the disc
/// this is arrives after the dialog is constructed — and a default row that
/// named a different game's archive is exactly the untrustworthy row that
/// function exists to prevent.
#[test]
fn the_launch_dialog_names_the_archive_this_story_will_actually_use() {
    let Some(disc) = disc() else {
        eprintln!("SKIP: the Masterpieces CD is absent");
        return;
    };
    for (entry, want) in [
        ("MAC/JOURNEY FOLDER/STORY.DATA", Some("MAC/JOURNEY FOLDER/CPIC.DATA")),
        ("PC/JOURNEY/JOURNEY.ZIP", Some("PC/JOURNEY/JOURNEY.MG1")),
    ] {
        let state = app::launch_options::LaunchOptionsState::new(
            "Journey",
            &disc,
            None,
            None,
            None,
            Some(blorb::medium::DiskImage::Hfs),
        )
        .on_disk_entry(Some(entry));
        assert_eq!(
            state.default_art.as_ref().map(|a| a.filename.as_str()),
            want,
            "the default-art row for {entry}"
        );
    }
}
