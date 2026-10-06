//! SQ-1720: a Scott Adams story inside a zip is titled by its MEMBER's stem.
//!
//! `scott_titles.tsv` is keyed on the bare lowercase stem (`secret`, `bond`).
//! A zip member's entry name carries an extension and maybe a directory, and it
//! was being handed to the lookup as-is, so every miss fell back to showing the
//! raw member name ("secret.dat") where the same bytes loose read "Top Secret
//! Adventure". The fixture bytes are one committed tiny Scott database; only the
//! member NAME carries the identity, which is the thing under test.

use std::io::Write as _;
use std::path::{Path, PathBuf};

const SCOTT: &[u8] = include_bytes!("../../../scott/tests/tiny_cave.dat");

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

fn rows(zip: &Path) -> Vec<app::picker::StoryEntry> {
    let data = app::scratch_dir("sq1720-data");
    app::picker::resolve_entries(zip, &app::data_roots::DataRoots::single(&data))
}

fn title_of(rows: &[app::picker::StoryEntry], entry: &str) -> String {
    rows.iter()
        .find(|r| r.meta.disk_entry.as_deref() == Some(entry))
        .unwrap_or_else(|| panic!("no row for {entry}: {rows:?}"))
        .title
        .clone()
}

#[test]
fn zip_members_are_titled_by_their_stem() {
    let dir = app::scratch_dir("sq1720-zip");
    let zip = dir.join("pack.zip");
    write_zip(
        &zip,
        &[("adv01.dat", SCOTT), ("sub/BOND.DAT", SCOTT), ("mystery_game.dat", SCOTT)],
    );
    let rows = rows(&zip);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(title_of(&rows, "adv01.dat"), "Adventureland");
    assert_eq!(title_of(&rows, "sub/BOND.DAT"), "James Bond Adventure");
    // An unknown member falls back to its stem: no extension, no directory.
    assert_eq!(title_of(&rows, "mystery_game.dat"), "mystery_game");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_game_title_uses_the_member_stem() {
    let dir = app::scratch_dir("sq1720-ingame");
    let t = app::picker::metadata_title_in(
        &dir.join("otheradv.zip"),
        &dir,
        "ifid",
        true,
        SCOTT,
        Some(app::picker::zip_member_stem("sub/secret.dat")),
    );
    assert_eq!(t.as_deref(), Some("Top Secret Adventure"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn otheradv_specimen_titles() {
    let zip: PathBuf = ["stories", "scott-dialects", "otheradv.zip"]
        .iter()
        .fold(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."), |p, s| p.join(s));
    if !zip.exists() {
        eprintln!("skip: {} absent", zip.display());
        return;
    }
    let rows = rows(&zip);
    let mut got: Vec<String> = rows.iter().map(|r| r.title.clone()).collect();
    got.sort();
    let mut want = vec![
        "James Bond Adventure",
        "Burglar's Adventure",
        "Gamma World",
        "Romulan Adventure",
        "Top Secret Adventure",
        "Marooned",
        "Miner's Adventure",
        "Undersea Conquest, Part I",
    ];
    want.sort();
    assert_eq!(got, want);
}
