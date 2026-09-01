//! `--import-metadata <tsv>`: land curated identifications in the sidecars the
//! picker already reads.
//!
//! The IFDB pass (`--fetch`) identifies a story by its IFID. Some stories have
//! no IFDB record under that IFID, and IFDB has no cover for many that do; a
//! person (or an agent) working from the IF Archive's descriptions, IFDB's
//! search, IFWiki or a competition's archive can settle those. This module is
//! how their answers get in: one TSV row per story, and for each row one of
//! three outcomes.
//!
//! - `ifdb_tuid` given: the story is fetched **by that IFDB id**, the same call
//!   the picker's `u` makes, and IFDB's record wins over anything else in the
//!   row. This is the common case for a story IFDB knows under a different
//!   IFID (a re-release, a zip whose IFID was computed from the archive).
//! - no tuid, `title` given: a *curated* sidecar is written from the row
//!   (`source = "curated"`), with the title, author, year, genre, language and
//!   description the row supplies. Nothing is invented: an empty column stays
//!   empty.
//! - only `cover_url` given (with or without a tuid the sidecar already has):
//!   the image is downloaded, checked to decode, saved as `cover.png`, and the
//!   existing sidecar's `cover` is pointed at it. A story that carries its own
//!   frontispiece is left alone, as the IFDB fetch leaves it.
//!
//! Columns are found by name in the header row, in any order; unknown columns
//! are ignored, so the file can carry `confidence`, `evidence` and whatever
//! else the person who made it found useful.

use std::path::{Path, PathBuf};

use crate::ifdb::{FetchError, FetchOutcome, MetadataSource};
use crate::story_info::{self, FetchedMeta};

/// One row of the import file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportRow {
    pub path: PathBuf,
    /// Which story on `path` when it is a zip or disk image holding several
    /// (the picker's `disk_entry`); `None` for a loose file.
    pub entry: Option<String>,
    pub ifdb_tuid: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
}

/// Parse the TSV: a header row naming columns, then one row per story. A row
/// without a `path` is an error; every other column is optional and read by
/// name. Fields are trimmed, and an empty field is `None`.
pub fn parse_tsv(text: &str) -> Result<Vec<ImportRow>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or("empty file")?;
    let cols: Vec<String> = header.split('\t').map(|c| c.trim().to_ascii_lowercase()).collect();
    let idx = |name: &str| cols.iter().position(|c| c == name);
    let path_col = idx("path").ok_or("the header has no `path` column")?;
    let entry_col = idx("entry");
    let (tuid, title, author, year, genre, language, description, cover) = (
        idx("ifdb_tuid"),
        idx("title"),
        idx("author"),
        idx("year"),
        idx("genre"),
        idx("language"),
        idx("description"),
        idx("cover_url"),
    );
    let mut rows = Vec::new();
    for (n, line) in lines.enumerate() {
        let fields: Vec<&str> = line.split('\t').collect();
        let get = |i: Option<usize>| -> Option<String> {
            let s = fields.get(i?)?.trim();
            (!s.is_empty()).then(|| s.to_string())
        };
        let Some(path) = get(Some(path_col)) else {
            return Err(format!("row {}: empty path", n + 2));
        };
        let tuid = get(tuid).and_then(|t| crate::ifdb::extract_tuid(&t));
        rows.push(ImportRow {
            path: PathBuf::from(path),
            entry: get(entry_col),
            ifdb_tuid: tuid,
            title: get(title),
            author: get(author),
            year: get(year),
            genre: get(genre),
            language: get(language),
            description: get(description),
            cover_url: get(cover),
        });
    }
    Ok(rows)
}

/// What became of one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowOutcome {
    /// Fetched from IFDB by the row's tuid; the sidecar is IFDB's record.
    FetchedById { title: String, cover: bool },
    /// A curated sidecar was written from the row itself.
    Curated { title: String, cover: bool },
    /// Only the cover changed on an existing sidecar.
    CoverAdded,
    /// Nothing to do, and why.
    Skipped(String),
    /// Something went wrong, and what.
    Failed(String),
}

/// Apply one row. `source` fetches by id and downloads covers (the picker's
/// own `IfdbClient` in production; a fake in tests).
pub fn import_row(row: &ImportRow, data_base: &Path, source: &dyn MetadataSource) -> RowOutcome {
    let Some(entry) = crate::picker::resolve_entry_from(&row.path, row.entry.as_deref(), data_base) else {
        return RowOutcome::Skipped(format!("{} is not a story lanthorn can open", label(row)));
    };
    let game_dir = entry.game_dir(data_base);
    let ifid = entry.meta.ifid.clone();

    if let Some(tuid) = &row.ifdb_tuid {
        match source.fetch_by_id(tuid) {
            Ok(FetchOutcome::Found(iff)) => {
                let cover = crate::fetch_worker::maybe_fetch_cover(source, &game_dir, &row.path, &iff);
                let title = iff.title.clone().unwrap_or_else(|| entry.title.clone());
                crate::fetch_worker::write_fetched(&game_dir, &ifid, crate::fetch_worker::found_meta(&iff, cover.clone()));
                // IFDB had no cover but the row names one: take it.
                let cover = if cover.is_none() {
                    fetch_cover_from(row.cover_url.as_deref(), source, &game_dir, &row.path)
                        .inspect(|c| set_cover(&game_dir, &ifid, c))
                } else {
                    cover
                };
                return RowOutcome::FetchedById { title, cover: cover.is_some() };
            }
            Ok(FetchOutcome::NotFound) if row.title.is_none() => {
                return RowOutcome::Skipped(format!("IFDB has no game {tuid}"));
            }
            Ok(FetchOutcome::NotFound) => {} // fall through to the curated row
            Err(FetchError::Transport(msg)) => return RowOutcome::Failed(msg),
        }
    }

    if let Some(title) = &row.title {
        // A curated line is for a story IFDB does not have; it must not
        // replace a record IFDB supplied (a collection zip whose members were
        // fetched by entry, for one). The cover in the line is still taken.
        if let Some(existing) = story_info::load(&game_dir, &ifid).and_then(|i| i.fetched) {
            if existing.source == "ifdb" && !existing.not_found {
                return match fetch_cover_from(row.cover_url.as_deref(), source, &game_dir, &row.path) {
                    Some(c) if existing.cover.is_none() => {
                        set_cover(&game_dir, &ifid, &c);
                        RowOutcome::CoverAdded
                    }
                    _ => RowOutcome::Skipped("already identified on IFDB".to_string()),
                };
            }
        }
        let cover = fetch_cover_from(row.cover_url.as_deref(), source, &game_dir, &row.path);
        let meta = FetchedMeta {
            scanned_at: crate::fetch_worker::now_rfc3339(),
            fetch_version: story_info::FETCH_VERSION,
            source: "curated".to_string(),
            title: Some(title.clone()),
            author: row.author.clone(),
            language: row.language.clone(),
            first_published: row.year.clone(),
            genre: row.genre.clone(),
            description: row.description.clone(),
            ifdb_tuid: None,
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            cover: cover.clone(),
            not_found: false,
        };
        crate::fetch_worker::write_fetched(&game_dir, &ifid, meta);
        return RowOutcome::Curated { title: title.clone(), cover: cover.is_some() };
    }

    if row.cover_url.is_some() {
        return match fetch_cover_from(row.cover_url.as_deref(), source, &game_dir, &row.path) {
            Some(c) => {
                set_cover(&game_dir, &ifid, &c);
                RowOutcome::CoverAdded
            }
            None => RowOutcome::Failed("the cover did not download or decode".to_string()),
        };
    }

    RowOutcome::Skipped("no tuid, title or cover_url in the row".to_string())
}

/// A row's story as the log names it: the path, and the entry when there is one.
fn label(row: &ImportRow) -> String {
    match &row.entry {
        Some(e) => format!("{} [{e}]", row.path.display()),
        None => row.path.display().to_string(),
    }
}

/// Download `url`, keep it only if it decodes as an image, save it beside the
/// sidecar as `cover.png`, and name it. `None` for no URL, a story with its
/// own frontispiece, or anything that fails.
fn fetch_cover_from(url: Option<&str>, source: &dyn MetadataSource, game_dir: &Path, path: &Path) -> Option<String> {
    let url = url?;
    if crate::cover::load_cover(path, None).is_some() {
        return None;
    }
    let bytes = source.fetch_cover(url).ok()?;
    crate::cover::decode(&bytes)?;
    std::fs::create_dir_all(game_dir).ok()?;
    let tmp = game_dir.join(format!(".cover.png.part-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).ok()?;
    if std::fs::rename(&tmp, game_dir.join("cover.png")).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    Some("cover.png".to_string())
}

/// Point an existing sidecar's `cover` at `cover`, leaving the rest as it is.
fn set_cover(game_dir: &Path, ifid: &str, cover: &str) {
    if let Some(mut info) = story_info::load(game_dir, ifid) {
        if let Some(f) = info.fetched.as_mut() {
            f.cover = Some(cover.to_string());
            let _ = story_info::save(game_dir, &info);
        }
    }
}

/// The whole file: parse, apply each row with a pause between network calls,
/// print one line per row, and return the process exit code (0 unless a row
/// failed).
pub fn run(tsv: &Path, data_base: &Path, source: &dyn MetadataSource, delay: std::time::Duration) -> i32 {
    let text = match std::fs::read_to_string(tsv) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("lanthorn: {}: {e}", tsv.display());
            return 2;
        }
    };
    let rows = match parse_tsv(&text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lanthorn: {}: {e}", tsv.display());
            return 2;
        }
    };
    let total = rows.len();
    let (mut by_id, mut curated, mut covers, mut skipped, mut failed) = (0, 0, 0, 0, 0);
    for (i, row) in rows.iter().enumerate() {
        let outcome = import_row(row, data_base, source);
        let word = match &outcome {
            RowOutcome::FetchedById { title, cover } => {
                by_id += 1;
                format!("{title}  (IFDB{})", if *cover { ", cover" } else { "" })
            }
            RowOutcome::Curated { title, cover } => {
                curated += 1;
                format!("{title}  (curated{})", if *cover { ", cover" } else { "" })
            }
            RowOutcome::CoverAdded => {
                covers += 1;
                "cover added".to_string()
            }
            RowOutcome::Skipped(why) => {
                skipped += 1;
                format!("skipped: {why}")
            }
            RowOutcome::Failed(why) => {
                failed += 1;
                format!("failed: {why}")
            }
        };
        println!("[{}/{total}] {}  {word}", i + 1, label(row));
        if row.ifdb_tuid.is_some() || row.cover_url.is_some() {
            std::thread::sleep(delay);
        }
    }
    println!("lanthorn: {by_id} fetched by IFDB id, {curated} curated, {covers} covers added, {skipped} skipped, {failed} failed");
    if failed > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifiction::{IFiction, IfdbExt};
    use std::sync::Mutex;

    /// A source that answers one tuid and serves one image.
    struct Fake {
        calls: Mutex<Vec<String>>,
    }
    impl MetadataSource for Fake {
        fn fetch(&self, _ifid: &str) -> Result<FetchOutcome, FetchError> {
            Ok(FetchOutcome::NotFound)
        }
        fn fetch_by_id(&self, tuid: &str) -> Result<FetchOutcome, FetchError> {
            self.calls.lock().unwrap().push(format!("id:{tuid}"));
            if tuid == "known0000tuid" {
                Ok(FetchOutcome::Found(Box::new(IFiction {
                    ifids: vec![],
                    title: Some("A Known Game".into()),
                    author: Some("Someone".into()),
                    language: None,
                    first_published: Some("1999".into()),
                    genre: None,
                    description: None,
                    ifdb: Some(IfdbExt {
                        tuid: tuid.into(),
                        link: None,
                        cover_url: None,
                        average_rating: None,
                        rating_count: None,
                    }),
                })))
            } else {
                Ok(FetchOutcome::NotFound)
            }
        }
        fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            self.calls.lock().unwrap().push(format!("cover:{url}"));
            if url.ends_with(".png") {
                let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 30, 30, 255]));
                let mut out = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
                Ok(out.into_inner())
            } else {
                Ok(b"<html>not an image</html>".to_vec())
            }
        }
    }

    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        buf[0x18] = 0x00; buf[0x19] = 0x60;
        buf[0x12..0x18].copy_from_slice(b"000000");
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf
    }

    fn scratch(tag: &str) -> PathBuf {
        crate::scratch_dir(&format!("metadata-import-{tag}"))
    }

    #[test]
    fn the_header_names_the_columns_in_any_order_and_extras_are_ignored() {
        let rows = parse_tsv("title\tconfidence\tpath\tifdb_tuid\tcover_url\tentry\nMy Game\thigh\t/lib/g.z5\thttps://ifdb.org/viewgame?id=abc123\t\t\n\t\t/lib/h.z5\t\thttps://x/y.png\tadv01.dat\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry, None);
        assert_eq!(rows[1].entry.as_deref(), Some("adv01.dat"), "a zip member is named by its entry");
        assert_eq!(rows[0].path, PathBuf::from("/lib/g.z5"));
        assert_eq!(rows[0].title.as_deref(), Some("My Game"));
        assert_eq!(rows[0].ifdb_tuid.as_deref(), Some("abc123"), "a viewgame URL is reduced to its id");
        assert_eq!(rows[0].cover_url, None);
        assert_eq!(rows[1].title, None);
        assert_eq!(rows[1].cover_url.as_deref(), Some("https://x/y.png"));
        assert!(parse_tsv("title\tauthor\nX\tY\n").is_err(), "no path column");
        assert!(parse_tsv("path\ttitle\n\tX\n").is_err(), "an empty path");
    }

    #[test]
    fn a_tuid_row_is_fetched_by_id_and_ifdb_wins_over_the_row() {
        let dir = scratch("by-id");
        let story = dir.join("g.z5");
        std::fs::write(&story, minimal_v3_story()).unwrap();
        let src = Fake { calls: Mutex::new(vec![]) };
        let row = ImportRow {
            path: story.clone(),
            ifdb_tuid: Some("known0000tuid".into()),
            title: Some("A Wrong Title".into()),
            cover_url: Some("https://covers.example/g.png".into()),
            ..Default::default()
        };
        let out = import_row(&row, &dir, &src);
        assert_eq!(out, RowOutcome::FetchedById { title: "A Known Game".into(), cover: true });
        let entry = crate::picker::resolve_entry(&story, &dir).unwrap();
        let info = story_info::load(&entry.game_dir(&dir), &entry.meta.ifid).unwrap();
        let f = info.fetched.unwrap();
        assert_eq!(f.source, "ifdb");
        assert_eq!(f.title.as_deref(), Some("A Known Game"));
        assert_eq!(f.cover.as_deref(), Some("cover.png"), "IFDB had no cover, so the row's was taken");
        assert!(entry.game_dir(&dir).join("cover.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_title_row_writes_a_curated_sidecar_and_a_bad_cover_is_not_kept() {
        let dir = scratch("curated");
        let story = dir.join("g.z5");
        std::fs::write(&story, minimal_v3_story()).unwrap();
        let src = Fake { calls: Mutex::new(vec![]) };
        let row = ImportRow {
            path: story.clone(),
            title: Some("Die Burg (The Castle)".into()),
            author: Some("Jemand".into()),
            year: Some("2004".into()),
            language: Some("German".into()),
            description: Some("A castle, a key, a dragon.".into()),
            cover_url: Some("https://covers.example/page.html".into()),
            ..Default::default()
        };
        let out = import_row(&row, &dir, &src);
        assert_eq!(out, RowOutcome::Curated { title: "Die Burg (The Castle)".into(), cover: false });
        let entry = crate::picker::resolve_entry(&story, &dir).unwrap();
        let f = story_info::load(&entry.game_dir(&dir), &entry.meta.ifid).unwrap().fetched.unwrap();
        assert_eq!(f.source, "curated");
        assert_eq!(f.author.as_deref(), Some("Jemand"));
        assert_eq!(f.first_published.as_deref(), Some("2004"));
        assert_eq!(f.cover, None, "an HTML page is not a cover");
        assert!(!entry.game_dir(&dir).join("cover.png").exists());
        // The picker reads it: the row's title is what the list shows.
        let listed = crate::picker::resolve_entry(&story, &dir).unwrap();
        assert_eq!(listed.title, "Die Burg (The Castle)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cover_only_row_updates_the_existing_sidecar() {
        let dir = scratch("cover-only");
        let story = dir.join("g.z5");
        std::fs::write(&story, minimal_v3_story()).unwrap();
        let src = Fake { calls: Mutex::new(vec![]) };
        // First an IFDB record without a cover.
        let first = ImportRow { path: story.clone(), ifdb_tuid: Some("known0000tuid".into()), ..Default::default() };
        assert_eq!(import_row(&first, &dir, &src), RowOutcome::FetchedById { title: "A Known Game".into(), cover: false });
        // Then a cover for it.
        let second = ImportRow { path: story.clone(), cover_url: Some("https://covers.example/g.png".into()), ..Default::default() };
        assert_eq!(import_row(&second, &dir, &src), RowOutcome::CoverAdded);
        let entry = crate::picker::resolve_entry(&story, &dir).unwrap();
        let f = story_info::load(&entry.game_dir(&dir), &entry.meta.ifid).unwrap().fetched.unwrap();
        assert_eq!(f.source, "ifdb", "the record itself is untouched");
        assert_eq!(f.cover.as_deref(), Some("cover.png"));
        // A curated title never replaces the IFDB record.
        let curated = ImportRow { path: story.clone(), title: Some("Collection".into()), ..Default::default() };
        assert!(matches!(import_row(&curated, &dir, &src), RowOutcome::Skipped(_)));
        let f = story_info::load(&entry.game_dir(&dir), &entry.meta.ifid).unwrap().fetched.unwrap();
        assert_eq!(f.title.as_deref(), Some("A Known Game"));
        // A row with nothing to apply is skipped, not an error.
        let empty = ImportRow { path: story.clone(), ..Default::default() };
        assert!(matches!(import_row(&empty, &dir, &src), RowOutcome::Skipped(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
