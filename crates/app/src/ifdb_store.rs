//! The shared IFDB store (SQ-1723): a game's fetched IFDB record and cover are
//! kept **once per IFDB entry**, in the catalogue, not once per copy.
//!
//! ```text
//! <catalogue>/ifdb/<tuid>.json    the record (a `FetchedMeta` plus a format version)
//! <catalogue>/ifdb/<tuid>.<ext>   the cover, in the bytes IFDB served (.png, .jpg, ...)
//! ```
//!
//! **This module is the only place those paths are spelled.** A copy's own
//! `<story-key>.save/info.json` keeps just its link state (which tuid it is
//! linked to; see [`crate::story_info`]), so two copies of one game share one
//! record and cover, a refresh through either is seen by both, and relinking
//! one leaves the other alone. The catalogue is shared by every player
//! (SQ-1676), so the store is too.
//!
//! The functions take the catalogue base, which for a copy's game directory is
//! its parent (`DataRoots::catalogue_dir(key)` is `<catalogue>/<key>.save`);
//! [`catalogue_of`] says so once. `ifdb/` cannot collide with a story folder,
//! which always ends in `.save`.
//!
//! A copy that holds no tuid (not linked, not found, or a curated row) keeps
//! nothing here: its cover, if any, stays `<game_dir>/cover.png`
//! ([`COPY_COVER`]), the pre-SQ-1723 location, which is also what an
//! un-adopted copy still has until [`crate::story_info`] moves it in.
//!
//! **"Newer wins"** (adoption and writes): two records for one tuid are compared
//! by the fetch timestamp they carry (`scanned_at`, RFC 3339). If either does
//! not parse, or they are equal, the file modification times decide; if those
//! tie too, the record already in the store stays.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::story_info::FetchedMeta;

/// The store's folder name inside the catalogue.
pub const DIR: &str = "ifdb";

/// A copy's own cover file: where a cover lives when there is no tuid to share
/// it under, and where every cover lived before the shared store.
pub const COPY_COVER: &str = "cover.png";

const RECORD_FORMAT_VERSION: u32 = 1;

/// The extensions a lookup tries, in order. A cover is written under the
/// extension of its real format (`image::ImageFormat::extensions_str`).
const COVER_EXTS: &[&str] = &["png", "jpg", "gif", "webp", "bmp", "tif", "avif"];

/// A tuid becomes a file name, so it must be only what IFDB ids are made of.
pub fn is_valid_tuid(tuid: &str) -> bool {
    !tuid.is_empty()
        && tuid.len() <= 64
        && tuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The tuid a record can be shared under: a found record with a usable id.
pub fn shared_tuid(meta: &FetchedMeta) -> Option<&str> {
    meta.ifdb_tuid.as_deref().filter(|t| !meta.not_found && is_valid_tuid(t))
}

/// The catalogue a copy's game directory sits in.
pub fn catalogue_of(game_dir: &Path) -> Option<&Path> {
    game_dir.parent().filter(|p| !p.as_os_str().is_empty())
}

pub fn store_dir(catalogue: &Path) -> PathBuf {
    catalogue.join(DIR)
}

pub fn record_path(catalogue: &Path, tuid: &str) -> PathBuf {
    store_dir(catalogue).join(format!("{tuid}.json"))
}

/// The stored cover for `tuid`, whichever format it is in.
pub fn cover_path(catalogue: &Path, tuid: &str) -> Option<PathBuf> {
    if !is_valid_tuid(tuid) {
        return None;
    }
    let dir = store_dir(catalogue);
    COVER_EXTS.iter().map(|e| dir.join(format!("{tuid}.{e}"))).find(|p| p.is_file())
}

#[derive(Serialize, Deserialize)]
struct SharedRecord {
    format_version: u32,
    #[serde(flatten)]
    meta: FetchedMeta,
}

/// The shared record for `tuid`, with `cover` naming the stored cover file (the
/// record on disk never carries it: the file's presence is the fact).
pub fn read_record(catalogue: &Path, tuid: &str) -> Option<FetchedMeta> {
    if !is_valid_tuid(tuid) {
        return None;
    }
    let raw = std::fs::read(record_path(catalogue, tuid)).ok()?;
    let rec: SharedRecord = serde_json::from_slice(&raw).ok()?;
    if rec.format_version != RECORD_FORMAT_VERSION {
        return None;
    }
    let mut meta = rec.meta;
    meta.cover = cover_path(catalogue, tuid)
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string));
    Some(meta)
}

fn record_bytes(meta: &FetchedMeta) -> io::Result<Vec<u8>> {
    let mut meta = meta.clone();
    meta.cover = None;
    let rec = SharedRecord { format_version: RECORD_FORMAT_VERSION, meta };
    Ok(serde_json::to_string_pretty(&rec)?.into_bytes())
}

/// Write `meta` as the record for `tuid`, atomically (two players may refresh at
/// once, SQ-1676). A write that would change nothing is skipped, which keeps the
/// file's modification time a true "last changed".
pub fn write_record(catalogue: &Path, tuid: &str, meta: &FetchedMeta) -> io::Result<()> {
    if !is_valid_tuid(tuid) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an IFDB id"));
    }
    let bytes = record_bytes(meta)?;
    let path = record_path(catalogue, tuid);
    if std::fs::read(&path).is_ok_and(|held| held == bytes) {
        return Ok(());
    }
    crate::storage::atomic_write(&path, &bytes)
}

/// [`write_record`] unless the store holds a strictly newer fetch of the same
/// entry (by `scanned_at`): a copy re-saving what it loaded earlier must not
/// undo a refresh another copy has made since.
pub fn write_record_unless_older(catalogue: &Path, tuid: &str, meta: &FetchedMeta) -> io::Result<()> {
    if let Some(held) = read_record(catalogue, tuid) {
        if let (Some(mine), Some(theirs)) = (parse_time(&meta.scanned_at), parse_time(&held.scanned_at)) {
            if mine < theirs {
                return Ok(());
            }
        }
    }
    write_record(catalogue, tuid, meta)
}

fn parse_time(s: &str) -> Option<jiff::Timestamp> {
    s.parse().ok()
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Is the candidate record (held in `candidate_file`) strictly newer than the
/// one the store holds? See the module docs for the rule.
fn is_newer(candidate: &FetchedMeta, candidate_file: &Path, held: &FetchedMeta, held_file: &Path) -> bool {
    if let (Some(a), Some(b)) = (parse_time(&candidate.scanned_at), parse_time(&held.scanned_at)) {
        if a != b {
            return a > b;
        }
    }
    match (modified(candidate_file), modified(held_file)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Store `bytes` as `tuid`'s cover, unchanged, under the extension of its real
/// format, replacing a cover held in another format. Returns the file name.
/// `InvalidData` for bytes that are not a recognised image.
pub fn write_cover(catalogue: &Path, tuid: &str, bytes: &[u8]) -> io::Result<String> {
    if !is_valid_tuid(tuid) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not an IFDB id"));
    }
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let format = image::guess_format(bytes).map_err(|_| bad("not an image"))?;
    let ext = format
        .extensions_str()
        .first()
        .copied()
        .filter(|e| COVER_EXTS.contains(e))
        .ok_or_else(|| bad("an image format with no cover extension"))?;
    let dir = store_dir(catalogue);
    let name = format!("{tuid}.{ext}");
    crate::storage::atomic_write(&dir.join(&name), bytes)?;
    for other in COVER_EXTS.iter().filter(|e| **e != ext) {
        let _ = std::fs::remove_file(dir.join(format!("{tuid}.{other}")));
    }
    Ok(name)
}

/// Keep a cover that has no tuid to share it under as the copy's own
/// [`COPY_COVER`]. Temp-then-rename so a crash cannot leave a truncated file.
pub fn write_copy_cover(game_dir: &Path, bytes: &[u8]) -> io::Result<String> {
    std::fs::create_dir_all(game_dir)?;
    crate::storage::atomic_write(&game_dir.join(COPY_COVER), bytes)?;
    Ok(COPY_COVER.to_string())
}

/// Persist a fetched cover: under `tuid` in the shared store when the copy has
/// one, else as the copy's own. Any pre-store file of the copy is adopted first
/// ([`crate::story_info::adopt_if_legacy`]) so it cannot be stranded or mistaken
/// for the new entry's. Returns the file name.
pub fn write_fetched_cover(game_dir: &Path, tuid: Option<&str>, bytes: &[u8]) -> io::Result<String> {
    crate::story_info::adopt_if_legacy(game_dir);
    match (tuid.filter(|t| is_valid_tuid(t)), catalogue_of(game_dir)) {
        (Some(tuid), Some(cat)) => write_cover(cat, tuid, bytes),
        _ => write_copy_cover(game_dir, bytes),
    }
}

/// The cover a copy's IFDB link provides, as the bytes IFDB served: the shared
/// entry's, else (no tuid, or the entry has none) the copy's own `cover.png`.
pub fn cover_bytes_for(game_dir: &Path) -> Option<Vec<u8>> {
    crate::story_info::adopt_if_legacy(game_dir);
    if let (Some(tuid), Some(cat)) = (crate::story_info::linked_tuid(game_dir), catalogue_of(game_dir)) {
        if let Some(bytes) = cover_path(cat, &tuid).and_then(|p| std::fs::read(p).ok()) {
            return Some(bytes);
        }
    }
    std::fs::read(game_dir.join(COPY_COVER)).ok()
}

/// Adoption (see the module docs): move a copy's own record `legacy` (read from
/// `info_file`) and its `cover.png` into the store under `tuid`. The newer fetch
/// wins; the copy's files are removed either way. Nothing is fetched.
pub(crate) fn adopt_legacy(
    catalogue: &Path,
    game_dir: &Path,
    tuid: &str,
    legacy: &FetchedMeta,
    info_file: &Path,
) -> io::Result<()> {
    let wins = match read_record(catalogue, tuid) {
        None => true,
        Some(held) => is_newer(legacy, info_file, &held, &record_path(catalogue, tuid)),
    };
    if wins {
        write_record(catalogue, tuid, legacy)?;
    }
    let own_cover = game_dir.join(COPY_COVER);
    if let Ok(bytes) = std::fs::read(&own_cover) {
        let held = cover_path(catalogue, tuid).is_some();
        let moved = if wins || !held { write_cover(catalogue, tuid, &bytes).is_ok() } else { false };
        if moved || held {
            let _ = std::fs::remove_file(&own_cover);
        }
    }
    Ok(())
}

/// Startup sweep (SQ-1724): adopt every copy directly under `catalogue` that
/// still holds its own IFDB record, whatever its IFID says today. Lazy adoption
/// never reaches a copy whose stored IFID no longer matches, or whose story is
/// not in the scanned library. One level only, no recursion, symlinks skipped;
/// `ifdb/` and anything not named `*.save` are ignored. Returns how many copies
/// were adopted. Idempotent: an adopted copy holds only a link afterwards.
pub fn adopt_catalogue(catalogue: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(catalogue) else { return 0 };
    let mut adopted = 0;
    for entry in rd.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().ends_with(".save") || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let dir = entry.path();
        let before = std::fs::read(crate::story_info::info_path(&dir)).ok();
        crate::story_info::adopt_if_legacy(&dir);
        if before.is_some() && std::fs::read(crate::story_info::info_path(&dir)).ok() != before {
            adopted += 1;
        }
    }
    adopted
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut out), format)
            .unwrap();
        out
    }

    use crate::story_info::{self, StoryInfo};

    fn meta(tuid: Option<&str>, title: &str, scanned_at: &str) -> FetchedMeta {
        FetchedMeta {
            scanned_at: scanned_at.into(),
            fetch_version: story_info::FETCH_VERSION,
            source: "ifdb".into(),
            title: Some(title.into()),
            author: None,
            language: None,
            first_published: None,
            genre: None,
            description: None,
            ifdb_tuid: tuid.map(str::to_string),
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            cover: None,
            not_found: false,
        }
    }

    fn info(ifid: &str, fetched: Option<FetchedMeta>) -> StoryInfo {
        StoryInfo { format_version: story_info::FORMAT_VERSION, ifid: ifid.into(), fetched, probe: None }
    }

    fn copy(cat: &Path, key: &str) -> PathBuf {
        crate::storage::game_dir(cat, key)
    }

    /// Write `info` the way every build before the shared store did: the whole
    /// record inside the copy's own `info.json`, its cover beside it.
    fn write_legacy(game_dir: &Path, info: &StoryInfo, cover: Option<&[u8]>) {
        std::fs::create_dir_all(game_dir).unwrap();
        std::fs::write(story_info::info_path(game_dir), serde_json::to_string_pretty(info).unwrap()).unwrap();
        if let Some(c) = cover {
            std::fs::write(game_dir.join(COPY_COVER), c).unwrap();
        }
    }

    fn store_files(cat: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(store_dir(cat))
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    fn title_of(game_dir: &Path, ifid: &str) -> Option<String> {
        story_info::load(game_dir, ifid)?.fetched?.title
    }

    #[test]
    fn two_copies_of_one_game_share_one_record_and_cover() {
        let cat = crate::scratch_dir("ifdb-store-two-copies");
        let (a, b) = (copy(&cat, "a.z5"), copy(&cat, "b.z5"));
        let m = meta(Some("t1"), "Zork", "2026-01-01T00:00:00Z");
        story_info::save(&a, &info("IFID-A", Some(m.clone()))).unwrap();
        story_info::save(&b, &info("IFID-B", Some(m))).unwrap();
        write_cover(&cat, "t1", &encoded(image::ImageFormat::Jpeg)).unwrap();

        // One record and one cover in the store, none in either copy.
        assert_eq!(store_files(&cat), ["t1.jpg", "t1.json"]);
        for d in [&a, &b] {
            assert!(!d.join(COPY_COVER).exists());
            let own = std::fs::read_to_string(story_info::info_path(d)).unwrap();
            assert!(!own.contains("Zork"), "a copy keeps only its link: {own}");
            assert!(own.contains("t1"));
        }
        let seen = story_info::load(&b, "IFID-B").unwrap().fetched.unwrap();
        assert_eq!((seen.title.as_deref(), seen.cover.as_deref()), (Some("Zork"), Some("t1.jpg")));

        // A refresh through A is seen by B, with nothing refetched for B.
        story_info::save(&a, &info("IFID-A", Some(meta(Some("t1"), "Zork I", "2026-02-01T00:00:00Z")))).unwrap();
        assert_eq!(title_of(&b, "IFID-B").as_deref(), Some("Zork I"));

        // A save of an older view (B loaded earlier) does not undo the refresh.
        story_info::save(&b, &info("IFID-B", Some(meta(Some("t1"), "Zork", "2026-01-01T00:00:00Z")))).unwrap();
        assert_eq!(title_of(&a, "IFID-A").as_deref(), Some("Zork I"));

        // Relinking A to another entry leaves B untouched.
        story_info::save(&a, &info("IFID-A", Some(meta(Some("t2"), "Zork II", "2026-03-01T00:00:00Z")))).unwrap();
        assert_eq!(title_of(&a, "IFID-A").as_deref(), Some("Zork II"));
        assert_eq!(title_of(&b, "IFID-B").as_deref(), Some("Zork I"));
        assert_eq!(story_info::linked_tuid(&a).as_deref(), Some("t2"));
        assert_eq!(story_info::linked_tuid(&b).as_deref(), Some("t1"));
        let _ = std::fs::remove_dir_all(&cat);
    }

    #[test]
    fn adoption_moves_a_copys_own_files_into_the_store_on_first_use() {
        let cat = crate::scratch_dir("ifdb-store-adopt");
        let a = copy(&cat, "a.z5");
        let cover = encoded(image::ImageFormat::Png);
        write_legacy(&a, &info("IFID-A", Some(meta(Some("t1"), "Zork", "2026-01-01T00:00:00Z"))), Some(&cover));

        // No network is involved anywhere: the record is simply read.
        let got = story_info::load(&a, "IFID-A").unwrap().fetched.unwrap();
        assert_eq!(got.title.as_deref(), Some("Zork"));
        assert_eq!(got.cover.as_deref(), Some("t1.png"));

        assert_eq!(store_files(&cat), ["t1.json", "t1.png"]);
        assert_eq!(std::fs::read(cover_path(&cat, "t1").unwrap()).unwrap(), cover);
        assert!(!a.join(COPY_COVER).exists(), "the per-copy cover was moved, not copied");
        assert!(!std::fs::read_to_string(story_info::info_path(&a)).unwrap().contains("Zork"));
        // Loading again changes nothing.
        assert_eq!(title_of(&a, "IFID-A").as_deref(), Some("Zork"));
        assert_eq!(store_files(&cat), ["t1.json", "t1.png"]);
        let _ = std::fs::remove_dir_all(&cat);
    }

    #[test]
    fn adoption_keeps_the_newer_fetch_and_removes_the_stale_files() {
        // Both orders, because "the first copy in wins" and "the last copy in
        // wins" are each the easy wrong answer.
        for older_first in [true, false] {
            let cat = crate::scratch_dir("ifdb-store-newer");
            let (old, new) = (copy(&cat, "old.z5"), copy(&cat, "new.z5"));
            let (old_cover, new_cover) = (encoded(image::ImageFormat::Png), encoded(image::ImageFormat::Jpeg));
            write_legacy(&old, &info("O", Some(meta(Some("t1"), "Old", "2026-01-01T00:00:00Z"))), Some(&old_cover));
            write_legacy(&new, &info("N", Some(meta(Some("t1"), "New", "2026-06-01T00:00:00Z"))), Some(&new_cover));

            if older_first {
                story_info::load(&old, "O");
                story_info::load(&new, "N");
            } else {
                story_info::load(&new, "N");
                story_info::load(&old, "O");
            }
            assert_eq!(title_of(&old, "O").as_deref(), Some("New"), "older_first={older_first}");
            assert_eq!(title_of(&new, "N").as_deref(), Some("New"));
            assert_eq!(store_files(&cat), ["t1.jpg", "t1.json"], "the newer cover alone remains");
            for d in [&old, &new] {
                assert!(!d.join(COPY_COVER).exists(), "stale per-copy cover removed");
            }
            let _ = std::fs::remove_dir_all(&cat);
        }
    }

    #[test]
    fn a_copy_with_no_tuid_keeps_nothing_in_the_store() {
        let cat = crate::scratch_dir("ifdb-store-notuid");
        let (nf, cur) = (copy(&cat, "nf.z5"), copy(&cat, "cur.z5"));
        // Not found: authoritative, per copy.
        let mut m = meta(None, "", "2026-01-01T00:00:00Z");
        m.title = None;
        m.not_found = true;
        story_info::save(&nf, &info("NF", Some(m.clone()))).unwrap();
        assert_eq!(story_info::load(&nf, "NF").unwrap().fetched, Some(m));
        // A curated row with a cover: both stay with the copy.
        story_info::save(&cur, &info("CUR", Some(meta(None, "Curated", "2026-01-01T00:00:00Z")))).unwrap();
        let png = encoded(image::ImageFormat::Png);
        write_fetched_cover(&cur, None, &png).unwrap();
        assert_eq!(cover_bytes_for(&cur), Some(png));
        assert_eq!(title_of(&cur, "CUR").as_deref(), Some("Curated"));
        assert!(!store_dir(&cat).exists(), "nothing was ever shared");
        let _ = std::fs::remove_dir_all(&cat);
    }

    #[test]
    fn a_wrong_games_link_is_per_copy_and_the_ifid_check_still_applies() {
        // The user pointing one copy at another IFDB page changes that copy's
        // link only; the other copy's link and the store entry it names stay.
        let cat = crate::scratch_dir("ifdb-store-relink");
        let (a, b) = (copy(&cat, "a.z5"), copy(&cat, "b.z5"));
        for d in [&a, &b] {
            story_info::save(d, &info("SAME", Some(meta(Some("wrong"), "Wrong Game", "2026-01-01T00:00:00Z")))).unwrap();
        }
        story_info::save(&a, &info("SAME", Some(meta(Some("right"), "Right Game", "2026-02-01T00:00:00Z")))).unwrap();
        assert_eq!(title_of(&a, "SAME").as_deref(), Some("Right Game"));
        assert_eq!(title_of(&b, "SAME").as_deref(), Some("Wrong Game"));
        assert_eq!(story_info::load(&a, "OTHER"), None, "a different game under the same name sees nothing");
        let _ = std::fs::remove_dir_all(&cat);
    }

    #[test]
    fn tuids_are_file_name_safe() {
        assert!(is_valid_tuid("k82q3libhff6ks8l"));
        for bad in ["", "../x", "a/b", "a.b", "a b"] {
            assert!(!is_valid_tuid(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_cover_keeps_its_bytes_and_its_real_extension() {
        let cat = crate::scratch_dir("ifdb-store-cover");
        let (p, j) = (encoded(image::ImageFormat::Png), encoded(image::ImageFormat::Jpeg));
        assert_eq!(write_cover(&cat, "t1", &p).unwrap(), "t1.png");
        assert_eq!(std::fs::read(cover_path(&cat, "t1").unwrap()).unwrap(), p);
        // A cover in another format replaces it rather than sitting beside it.
        assert_eq!(write_cover(&cat, "t1", &j).unwrap(), "t1.jpg");
        assert_eq!(std::fs::read(cover_path(&cat, "t1").unwrap()).unwrap(), j);
        assert!(!store_dir(&cat).join("t1.png").exists());
        assert!(write_cover(&cat, "t1", b"<html>").is_err(), "junk is not a cover");
        assert!(write_cover(&cat, "../t1", &p).is_err());
    }

    #[test]
    fn the_sweep_adopts_copies_the_lazy_path_never_reaches() {
        let cat = crate::scratch_dir("ifdb-store-sweep");
        let (stale, none, nf) = (copy(&cat, "stale.z5"), copy(&cat, "none.z5"), copy(&cat, "nf.z5"));
        let cover = encoded(image::ImageFormat::Png);
        // An IFID nothing computes today: `load` would reject it before adopting.
        write_legacy(&stale, &info("ZCODE-8202- - 31 -0A20", Some(meta(Some("t1"), "Zork", "2026-01-01T00:00:00Z"))), Some(&cover));
        write_legacy(&none, &info("N", Some(meta(None, "Curated", "2026-01-01T00:00:00Z"))), Some(&cover));
        let mut m = meta(None, "", "2026-01-01T00:00:00Z");
        m.not_found = true;
        write_legacy(&nf, &info("NF", Some(m)), None);
        let (none_before, nf_before) =
            (std::fs::read(story_info::info_path(&none)).unwrap(), std::fs::read(story_info::info_path(&nf)).unwrap());

        assert_eq!(adopt_catalogue(&cat), 1);

        assert_eq!(store_files(&cat), ["t1.json", "t1.png"]);
        assert_eq!(std::fs::read(cover_path(&cat, "t1").unwrap()).unwrap(), cover);
        assert_eq!(read_record(&cat, "t1").unwrap().title.as_deref(), Some("Zork"));
        let own = std::fs::read_to_string(story_info::info_path(&stale)).unwrap();
        assert!(own.contains("link") && !own.contains("fetched"), "{own}");
        assert!(!stale.join(COPY_COVER).exists());
        assert_eq!(std::fs::read(story_info::info_path(&none)).unwrap(), none_before);
        assert_eq!(std::fs::read(story_info::info_path(&nf)).unwrap(), nf_before);
        assert!(none.join(COPY_COVER).exists());
        // Second run: nothing left to adopt.
        assert_eq!(adopt_catalogue(&cat), 0);
        let _ = std::fs::remove_dir_all(&cat);
    }

    #[test]
    fn the_sweep_keeps_a_newer_store_record_and_removes_the_stale_copy_files() {
        let cat = crate::scratch_dir("ifdb-store-sweep-newer");
        write_record(&cat, "t1", &meta(Some("t1"), "New", "2026-06-01T00:00:00Z")).unwrap();
        let a = copy(&cat, "a.z5");
        write_legacy(&a, &info("OLD-IFID", Some(meta(Some("t1"), "Old", "2026-01-01T00:00:00Z"))), Some(&encoded(image::ImageFormat::Png)));
        adopt_catalogue(&cat);
        assert_eq!(read_record(&cat, "t1").unwrap().title.as_deref(), Some("New"));
        assert!(!a.join(COPY_COVER).exists());
        let own = std::fs::read_to_string(story_info::info_path(&a)).unwrap();
        assert!(!own.contains("Old"), "{own}");
        let _ = std::fs::remove_dir_all(&cat);
    }
}
