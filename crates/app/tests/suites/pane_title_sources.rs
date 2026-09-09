//! SQ-0766: the story pane's border title must name the game the way the story
//! browser's list does.
//!
//! The pane used to resolve its title from a three-tier cascade — an IFID
//! known-title table, a banner heuristic, then the filename stem — and fell to
//! the stem whenever those narrow sources missed. The browser, meanwhile,
//! already resolved the same stories correctly from real metadata, so the same
//! game read one way in the list and another in the pane: `anchor` for
//! Anchorhead, `photo201` for Photopia, `arrow1` for Arrow of Death Part 1,
//! `cragne` for Cragne Manor.
//!
//! The fix is one shared resolver, [`app::picker::metadata_title`] — the
//! container's own `IFmd` chunk, then the fetched IFDB sidecar, then the bundled
//! tables — asked by both, with the banner heuristic demoted below it and the
//! stem left as the genuine last resort.
//!
//! These drive real story files, so they skip vacuously when `stories/` (which
//! is gitignored) is absent. The fetched sidecars are WRITTEN BY THE TEST into a
//! temp data base rather than read out of the developer's cache, so the tiers
//! are exercised deterministically.

use std::path::{Path, PathBuf};

use app::session::{format_pane_title, resolve_title};

use crate::fixture_paths::fixture_path;


fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: gitignored story missing at {}", p.display());
        None
    }
}

fn tmp_base(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq0766-{tag}"))
}

/// The identity and engine the app derives at boot: the IFID of the EXECUTABLE
/// mounted out of the file (not the container's raw bytes), plus whether that
/// executable is a Scott database, plus whether the file was a disk image.
fn identity(path: &Path) -> (String, bool, bool) {
    let (loaded, disk_image) = app::hints::load_mounted_story(path).expect("story must load");
    let is_scott = matches!(loaded, app::hints::LoadedStory::Scott(_));
    (app::ifid::compute_ifid(loaded.bytes()), is_scott, disk_image.is_some())
}

/// The executable bytes `metadata_title` needs (SQ-1469): the same bytes
/// `identity` derives the IFID from, mounted fresh since `identity` does not
/// hand its own back out.
fn exec_bytes(path: &Path) -> Vec<u8> {
    let (loaded, _) = app::hints::load_mounted_story(path).expect("story must load");
    loaded.bytes().to_vec()
}

/// Write the fetched-IFDB sidecar the story browser reads, into `data_base`.
fn seed_sidecar(data_base: &Path, path: &Path, ifid: &str, title: &str) {
    let game_dir = app::storage::game_dir(data_base, &app::storage::story_key_at(path));
    let info = app::story_info::StoryInfo {
        format_version: app::story_info::FORMAT_VERSION,
        ifid: ifid.to_string(),
        fetched: Some(app::story_info::FetchedMeta {
            scanned_at: "2026-08-11T00:00:00Z".into(),
            fetch_version: app::story_info::FETCH_VERSION,
            source: "ifdb".into(),
            title: Some(title.to_string()),
            author: None,
            language: None,
            first_published: None,
            genre: None,
            description: None,
            ifdb_tuid: None,
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            cover: None,
            not_found: false,
        }),
        probe: None,
    };
    app::story_info::save(&game_dir, &info).expect("sidecar must write");
}

/// The whole pipeline for one story, as `startup::boot_story` runs it: identity
/// → shared metadata resolver → `resolve_title` → `format_pane_title`.
fn pane_title(path: &Path, data_base: &Path, banner_title: Option<&str>) -> String {
    let (ifid, is_scott, disk_image) = identity(path);
    let meta = app::picker::metadata_title(path, data_base, &ifid, is_scott, &exec_bytes(path));
    let name = resolve_title(None, meta.as_deref(), banner_title, path);
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    format_pane_title(&name, filename, disk_image)
}

/// The top tier: a Glulx/Z-code blorb that carries its own Treaty-of-Babel
/// `IFmd` chunk names itself, with nothing fetched and no network. The pane
/// showed `cragne` for a file that has been carrying "Cragne Manor" inside it
/// all along — the same defect a fourth time, on Glulx.
///
/// FALSIFY by dropping the `metadata` tier from `resolve_title`:
/// `cragne.gblorb: the container's own metadata must name it, left: "cragne"`.
#[test]
fn a_containers_own_ifmd_chunk_names_the_game() {
    let cases = [
        ("cragne.gblorb", "Cragne Manor"),
        ("the-impossible-bottle.zblorb.blorb", "The Impossible Bottle"),
    ];
    let base = tmp_base("ifmd"); // deliberately empty: no sidecar, no fetch
    for (file, title) in cases {
        let Some(path) = story(file) else { continue };
        assert_eq!(
            pane_title(&path, &base, None),
            format!("{title} ({file})"),
            "{file}: the container's own metadata must name it"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// PART C and PART D: a fetched IFDB sidecar is the tier the pane never asked
/// for, and for these two it is the only one that knows the game — neither has a
/// usable banner, and `photo201.blb` carries no `IFmd` chunk (only an `RIdx`),
/// so before the fix both showed their filename stem.
///
/// FALSIFY by dropping the `metadata` tier from `resolve_title`:
/// `anchor.z8: expected Anchorhead, got anchor` — the user's report verbatim.
#[test]
fn a_fetched_sidecar_names_the_game_the_browser_lists() {
    let cases = [
        ("anchor.z8", "Anchorhead"),  // Z-machine, boots into a paged title plate
        ("photo201.blb", "Photopia"), // Glulx blorb, RIdx only — no IFmd
    ];
    let base = tmp_base("fetched");
    for (file, title) in cases {
        let Some(path) = story(file) else { continue };
        let (ifid, is_scott, _) = identity(&path);

        // Premise: with nothing seeded, the pane falls to the stem — the bug.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap().to_string();
        assert_eq!(
            app::picker::metadata_title(&path, &base, &ifid, is_scott, &exec_bytes(&path)),
            None,
            "{file}: premise — no metadata source knows it yet"
        );
        assert_eq!(pane_title(&path, &base, None), stem, "{file}: premise — bare stem");

        // Seed the sidecar the browser reads, and the pane must now agree with it.
        seed_sidecar(&base, &path, &ifid, title);
        assert_eq!(
            app::picker::metadata_title(&path, &base, &ifid, is_scott, &exec_bytes(&path)).as_deref(),
            Some(title),
            "{file}: the shared resolver must read the fetched sidecar"
        );
        let got = pane_title(&path, &base, None);
        assert!(got.starts_with(title), "{file}: expected {title}, got {got}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The sidecar is keyed by IFID: one belonging to a different story is ignored,
/// so a stale or mismatched cache can never rename the game being played.
#[test]
fn a_sidecar_for_a_different_ifid_is_ignored() {
    let Some(path) = story("anchor.z8") else { return };
    let base = tmp_base("wrongifid");
    let (ifid, is_scott, _) = identity(&path);
    seed_sidecar(&base, &path, "ZCODE-1-000000-0000", "Not This Game");
    assert_eq!(
        app::picker::metadata_title(&path, &base, &ifid, is_scott, &exec_bytes(&path)),
        None
    );
    assert_eq!(pane_title(&path, &base, None), "anchor");
    let _ = std::fs::remove_dir_all(&base);
}

/// PART B: the bundled Scott table (`scott_titles.tsv`, keyed by filename stem)
/// is the offline tier the browser has always used and the pane never did.
/// Nothing is seeded here — no sidecar, no network — and the pane must still
/// name the game.
///
/// FALSIFY by dropping the Scott tier from `picker::bundled_title` (leaving only
/// the IFID table): `arrow1.blb: expected Arrow of Death Part 1, got arrow1` —
/// the user's report verbatim.
#[test]
fn the_bundled_scott_table_names_a_scott_story_offline() {
    let cases = [("arrow1.blb", "Arrow of Death Part 1"), ("adv01.dat", "Adventureland")];
    let base = tmp_base("scott");
    for (file, title) in cases {
        let Some(path) = story(file) else { continue };
        let (_, is_scott, _) = identity(&path);
        assert!(is_scott, "{file}: must mount as a Scott database");
        assert!(
            pane_title(&path, &base, None).starts_with(title),
            "{file}: expected {title}, got {}",
            pane_title(&path, &base, None)
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// SQ-1469: a Commodore 64 *Mysterious Adventures* program file opened
/// DIRECTLY — no disk entry to name it, unlike the `.d64` route — must show
/// its release title in the pane the same way the disk route does, not the
/// bare CBM filename stem (`BATON`). `bundled_title` identifies it by content
/// checksum now, so `scott_titles.tsv`'s IF-Archive-filename keys (which
/// never match a disk's own spelling) are no longer the only offline tier.
#[test]
fn a_c64_mysterious_program_file_opened_directly_still_gets_its_release_title() {
    let Some(path) = story("scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg") else { return };
    let base = tmp_base("c64-direct");
    let (_, is_scott, _) = identity(&path);
    assert!(is_scott, "BATON.prg must mount as a Scott database");
    let title = pane_title(&path, &base, None);
    assert!(
        title.starts_with("The Golden Baton"),
        "expected The Golden Baton…, got {title} — the pre-fix defect showed the bare stem BATON"
    );
    let _ = std::fs::remove_dir_all(&base);
}

/// The IFID known-title table still works, and still outranks a banner — the
/// tier simply moved into the shared resolver.
#[test]
fn the_bundled_ifid_table_still_names_an_infocom_story() {
    let Some(path) = story("zork1-r88-s840726.z3") else { return };
    let base = tmp_base("known");
    let title = pane_title(&path, &base, Some("SOMETHING ELSE"));
    assert!(title.starts_with("Zork I"), "expected Zork I…, got {title}");
    let _ = std::fs::remove_dir_all(&base);
}

/// PART A: a disk image is a different RELEASE, so the pane always names the
/// `.adf` — even where the box-spelled filename normalizes onto the very title
/// the metadata gives, which is precisely when the old comparison hid it.
///
/// FALSIFY by dropping `!disk_image &&` from `format_pane_title`:
/// `left: "Arthur: The Quest for Excalibur"` against the expected
/// `"Arthur: The Quest for Excalibur (Arthur - The Quest for Excalibur.adf)"` —
/// the medium gone from the pane, which is the report.
#[test]
fn a_disk_image_is_always_named_in_the_pane() {
    let cases = [
        ("Arthur - The Quest for Excalibur.adf", "Arthur: The Quest for Excalibur"),
        ("Journey - The Quest Begins.adf", "Journey: The Quest Begins"),
    ];
    let base = tmp_base("adf");
    for (file, title) in cases {
        let Some(path) = story(file) else { continue };
        let (ifid, _, disk_image) = identity(&path);
        assert!(disk_image, "{file}: must mount as a disk image");
        seed_sidecar(&base, &path, &ifid, title);
        assert_eq!(pane_title(&path, &base, None), format!("{title} ({file})"), "{file}");
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The bare `.z6` build of the same game keeps its plain title — the disk-image
/// rule must not leak into ordinary story files.
#[test]
fn a_bare_story_file_is_unaffected_by_the_disk_image_rule() {
    let Some(path) = story("journey-r83-s890706.z6") else { return };
    let base = tmp_base("bare");
    let (ifid, _, disk_image) = identity(&path);
    assert!(!disk_image);
    seed_sidecar(&base, &path, &ifid, "Journey");
    // Stem is release/serial-suffixed, so it differs and is disclosed on merit.
    assert_eq!(pane_title(&path, &base, None), "Journey (journey-r83-s890706.z6)");
    let _ = std::fs::remove_dir_all(&base);
}
