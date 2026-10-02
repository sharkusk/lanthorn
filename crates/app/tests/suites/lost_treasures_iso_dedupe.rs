//! SQ-1517: two picker defects found by pointing the story browser at the real
//! *Lost Treasures of Infocom* CD-ROM compilation, `treasures/ISOs/`.
//!
//! Fixtures: `treasures/ISOs/LostTreasures1.iso` (release I, 40 of its own
//! story files) and `LostTreasures2.iso` (release II, 28). Both gitignored;
//! symlink `treasures/` from the main checkout. `LostTreasures1`/`2`'s
//! filenames alone satisfy `disk_set::group`'s multi-volume naming rule, so
//! `app::picker::scan_stories` treats the pair as one shelf and folds
//! duplicate BUILDS across them exactly as it would fold a genuine
//! multi-floppy release's volumes — which is where both bugs below live.
//!
//! # Bug 1 — a hybrid volume's own machines collapsed into one row
//!
//! `LostTreasures1.iso` is a hybrid disc: `MAC/BEYOND ZORK` and
//! `PC/DATA/BEYONDZO.DAT` are the SAME byte-identical build
//! (`ZCODE-57-871221-C5AD`) on two different machines, which
//! `picker::dedupe_within_a_volume` is specifically keyed (on the machine, not
//! just the IFID) to keep apart. But because the two ISOs also satisfy the
//! set-naming rule, `picker::dedupe_within_sets` ran over the SAME rows
//! afterwards keyed on IFID alone — re-folding a pair that never crossed a
//! volume boundary at all, and silently dropping the DOS row. `MAC/CUTTHROATS`
//! / `PC/CUTTHROA/CUTTHROA.DAT` is the same shape.
//!
//! # Bug 2 — a nested disk image's save/metadata key disagreed with itself
//!
//! *Lost Treasures I*'s `PC/ZORK0/ZORK0.ZIP` is not a plain file on the
//! Iso9660 filesystem — it is a Fat12 floppy dump PACKED inside the CD, so its
//! own machine (`Fat12Dos`) differs from the container's own format
//! (`Iso9660`). `cli_host::storage::mounted_build` computed the save/metadata
//! directory's medium from `DiskImage::detect` on the CONTAINER's raw bytes,
//! while the picker's own row (via `hints::mounted_stories`'s `image_for`)
//! keys on the STORY's own machine. The two directories differ only for this
//! one game on the whole disc, so an IFDB fetch wrote Zork Zero DOS's
//! author/year/genre/description into a directory the picker never reads
//! back from — "no metadata", however many times `f`/`r` ran.

use std::path::PathBuf;

fn treasures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../treasures/ISOs")
}

fn iso1() -> PathBuf {
    treasures_dir().join("LostTreasures1.iso")
}

/// `None` (with a SKIP line) when the gitignored fixture is absent — the CI-safe
/// pattern every real-media suite here uses.
fn present() -> bool {
    if iso1().exists() {
        return true;
    }
    eprintln!("SKIP: gitignored disc missing at {}", iso1().display());
    false
}

fn data_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq1517-{tag}"))
}

/// **Bug 1's defect, off the real disc.** `Beyond Zork: The Coconut of
/// Quendor` r57/s871221 sits on `LostTreasures1.iso` twice, once per machine
/// (`MAC/BEYOND ZORK`, `PC/DATA/BEYONDZO.DAT`) — see the module doc. Both rows
/// must survive the scan.
///
/// Non-vacuity: the row count is checked against the union of both discs' own
/// raw story counts (`zvm-cli`'s `COMPILATION_DISCS` pins 40 and 28) so a scan
/// that silently found nothing cannot read as a pass, and the machine check
/// below cannot be satisfied by a single row that happens to carry `None`.
///
/// FALSIFICATION: drop the `disk_image` half of `dedupe_within_sets`'s key
/// (keep only `(set_idx, ifid)`) and this fails — only one of the two rows
/// below survives.
#[test]
fn a_hybrid_volumes_own_machines_survive_the_cross_volume_fold() {
    if !present() {
        return;
    }
    let dir = treasures_dir();
    let base = data_base("beyondzork");
    let rows = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&base));
    assert!(
        rows.len() >= 60,
        "expected close to the union of both discs' own 40+28 raw stories, got {}: \
         a collapsed count this low means rows are being folded away",
        rows.len()
    );

    let beyond: Vec<&app::picker::StoryEntry> =
        rows.iter().filter(|r| r.meta.ifid == "ZCODE-57-871221-C5AD").collect();
    assert_eq!(
        beyond.len(),
        2,
        "Beyond Zork r57/s871221 should keep one row per machine on \
         LostTreasures1.iso, got: {:?}",
        beyond.iter().map(|r| (r.meta.disk_image, r.meta.disk_entry.as_deref())).collect::<Vec<_>>()
    );
    let has_mac = beyond.iter().any(|r| r.meta.disk_image == Some(app::hints::DiskImage::Hfs));
    let has_dos = beyond.iter().any(|r| r.meta.disk_image == Some(app::hints::DiskImage::Iso9660));
    assert!(has_mac && has_dos, "expected one Mac (Hfs) row and one DOS (Iso9660) row: {beyond:?}");
}

/// **Bug 2's defect, off the real disc.** Zork Zero's DOS build
/// (r393/s890714, IFID `ZCODE-393-890714-791C`) is `PC/ZORK0/ZORK0.ZIP` on
/// `LostTreasures1.iso` — a nested Fat12 floppy dump, so its per-story machine
/// (what the picker's own row carries) is `Fat12Dos`, not the container's own
/// `Iso9660`. The fetch worker's save/metadata directory must be keyed the
/// same way the picker's row already is, or a completed fetch writes into a
/// directory the list never reads back from.
///
/// Non-vacuity: the `expect` below fails outright if the disc's story roster
/// or its `disk_entry` naming ever changes, rather than silently comparing
/// two `None`s.
///
/// FALSIFICATION: put `DiskImage::detect(&raw)`'s answer back in place of
/// `disk.image_for(&chosen.name)` inside `cli_host::storage::mounted_build`
/// and this fails — the two keys below diverge (`…-Iso9660...` vs
/// `…-Fat12Dos...`).
#[test]
fn a_nested_disk_images_save_key_matches_the_pickers_own_row() {
    if !present() {
        return;
    }
    let dir = treasures_dir();
    let base = data_base("zork0key");
    let rows = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&base));
    let dos_zork0 = rows
        .iter()
        .find(|r| r.meta.ifid == "ZCODE-393-890714-791C")
        .expect("LostTreasures1.iso carries Zork Zero's DOS build (r393/s890714) as PC/ZORK0/ZORK0.ZIP");
    assert_eq!(
        dos_zork0.meta.disk_image,
        Some(app::hints::DiskImage::Fat12Dos),
        "the nested floppy's own machine, from image_for"
    );
    let picker_key = dos_zork0.story_key();
    let fetch_worker_key =
        app::storage::story_key_at_from(&dos_zork0.path, dos_zork0.meta.disk_entry.as_deref());
    assert_eq!(
        picker_key, fetch_worker_key,
        "the fetch worker's save/metadata directory must name the same story \
         the picker's row does, or a completed fetch is invisible to the list"
    );
}
