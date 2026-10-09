//! Does the library already hold the game an IFDB record describes? (SQ-1755)
//!
//! A game is the same game when its IFID is one of the record's `<ifid>`s or
//! its IFDB tuid is the record's — wherever in the library it lives. The IFID
//! is read off the story file itself (`StoryEntry::meta.ifid`), so a library
//! whose own IFDB details were never fetched is still matched.
//!
//! [`library_holds`] walks the library the way [`crate::picker::index_library`]
//! does and opens every story file, so a host must call it off its UI thread;
//! [`matching_entries`] is the pure half, for a host that already holds an
//! index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::data_roots::DataRoots;
use crate::ifdb_search::DownloadOption;
use crate::ifiction::IFiction;
use crate::picker::StoryEntry;

/// The picker's finished library index, shared with the IFDB search worker
/// (SQ-1757). `None` while the picker is still indexing; the picker stores the
/// completed entries once, and the worker then matches IFIDs against them
/// instead of re-reading and re-hashing every story file.
pub type LibraryIndex = Arc<Mutex<Option<Arc<Vec<StoryEntry>>>>>;

/// Paths of the entries in `entries` that are the game `record` describes:
/// IFID in the record's list (case-insensitive, trimmed) or the same IFDB
/// tuid. Folder rows are skipped. Sorted and deduplicated.
pub fn matching_entries(entries: &[StoryEntry], record: &IFiction) -> Vec<PathBuf> {
    let ifids: Vec<String> = record.ifids.iter().map(|i| i.trim().to_ascii_lowercase()).collect();
    let tuid = record.ifdb.as_ref().map(|e| e.tuid.trim()).filter(|t| !t.is_empty());
    let mut out: Vec<PathBuf> = entries
        .iter()
        .filter(|e| !e.is_folder())
        .filter(|e| {
            let id = e.meta.ifid.trim().to_ascii_lowercase();
            (!id.is_empty() && ifids.contains(&id))
                || matches!((tuid, e.meta.ifdb_tuid.as_deref()), (Some(t), Some(m)) if m.trim() == t)
        })
        .map(|e| e.path.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Every story file under `root` (all subfolders) that is the game `record`
/// describes. Reads each story file: call it on a worker thread.
pub fn library_holds(root: &Path, roots: &DataRoots, record: &IFiction) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in crate::picker::library_dirs(root) {
        out.extend(matching_entries(&crate::picker::scan_stories(&dir, roots), record));
    }
    out.sort();
    out.dedup();
    out
}

/// For each of `options`, the paths among `files` whose file name equals the
/// option's `filename`, ASCII case-insensitively on every platform (SQ-1757).
/// Names only. Each inner list is sorted and deduplicated; the outer one is
/// parallel to `options`. One pass over `files`, not a nested loop.
pub fn option_paths(options: &[DownloadOption], files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    let mut by_name: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    for f in files {
        if let Some(name) = f.file_name() {
            by_name.entry(name.to_string_lossy().to_ascii_lowercase()).or_default().push(f);
        }
    }
    options
        .iter()
        .map(|o| {
            let mut v: Vec<PathBuf> = by_name
                .get(&o.filename.to_ascii_lowercase())
                .map(|ps| ps.iter().map(|p| (*p).clone()).collect())
                .unwrap_or_default();
            v.sort();
            v.dedup();
            v
        })
        .collect()
}

/// Every regular file (any extension) directly inside any of `dirs`. Directory
/// listings only: no file is opened.
fn files_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            // `metadata` follows symlinks, so a linked file counts and a linked
            // directory (already walked via `library_dirs`) does not.
            if std::fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false) {
                out.push(path);
            }
        }
    }
    out
}

/// For each of `options`, where under `root` (all subfolders) a file of that
/// name already sits (SQ-1757). Directory listings only, so it is cheap next to
/// [`library_holds`], but still a filesystem walk: call it off the UI thread.
pub fn options_in_library(root: &Path, options: &[DownloadOption]) -> Vec<Vec<PathBuf>> {
    option_paths(options, &files_in(&crate::picker::library_dirs(root)))
}

/// What the search worker asks of the library for one resolved game, in a
/// SINGLE [`crate::picker::library_dirs`] traversal: the story files that are
/// the game (`.0`, as [`library_holds`]) and, per option, the files already
/// carrying its name (`.1`, as [`options_in_library`]).
///
/// With `index` (the picker's finished index) the IFID check reads those
/// entries and opens no story file at all; without it, it falls back to
/// scanning each folder, which reads and hashes every story.
pub fn resolve_in_library(
    root: &Path,
    roots: &DataRoots,
    record: Option<&IFiction>,
    options: &[DownloadOption],
    index: Option<&[StoryEntry]>,
) -> (Vec<PathBuf>, Vec<Vec<PathBuf>>) {
    let dirs = crate::picker::library_dirs(root);
    let named = option_paths(options, &files_in(&dirs));
    let held = match (record, index) {
        (None, _) => Vec::new(),
        (Some(r), Some(entries)) => matching_entries(entries, r),
        (Some(r), None) => {
            let mut out = Vec::new();
            for dir in &dirs {
                out.extend(matching_entries(&crate::picker::scan_stories(dir, roots), r));
            }
            out.sort();
            out.dedup();
            out
        }
    };
    (held, named)
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use crate::ifiction::IfdbExt;

    fn entry(path: &str, ifid: &str, tuid: Option<&str>) -> StoryEntry {
        let mut e = StoryEntry::folder(PathBuf::from(path), "x");
        e.kind = crate::picker::RowKind::Story;
        e.meta.ifid = ifid.to_string();
        e.meta.ifdb_tuid = tuid.map(str::to_string);
        e
    }

    fn record(ifids: &[&str], tuid: Option<&str>) -> IFiction {
        IFiction {
            ifids: ifids.iter().map(|s| s.to_string()).collect(),
            ifdb: tuid.map(|t| IfdbExt {
                tuid: t.to_string(),
                link: None,
                cover_url: None,
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_ifid_match_in_a_subfolder_is_found() {
        let es = [entry("/lib/glulx/a.ulx", "GLULX-1", None), entry("/lib/b.z5", "ZCODE-2", None)];
        assert_eq!(
            matching_entries(&es, &record(&["ZCODE-9", "GLULX-1"], None)),
            vec![PathBuf::from("/lib/glulx/a.ulx")]
        );
    }

    #[test]
    fn ifids_compare_case_insensitively_and_trimmed() {
        let es = [entry("/lib/a.ulx", "uuid-ab", None)];
        assert_eq!(matching_entries(&es, &record(&[" UUID-AB "], None)).len(), 1);
    }

    #[test]
    fn a_tuid_match_needs_no_ifid_overlap() {
        let es = [entry("/lib/a.z5", "ZCODE-1", Some("t123"))];
        assert_eq!(matching_entries(&es, &record(&["ZCODE-OTHER"], Some("t123"))).len(), 1);
    }

    #[test]
    fn no_overlap_matches_nothing() {
        let es = [entry("/lib/a.z5", "ZCODE-1", Some("t1"))];
        assert!(matching_entries(&es, &record(&["ZCODE-2"], Some("t2"))).is_empty());
        assert!(matching_entries(&es, &record(&[], None)).is_empty());
    }

    #[test]
    fn an_empty_ifid_never_matches() {
        let es = [entry("/lib/a.z5", "", None)];
        assert!(matching_entries(&es, &record(&[""], None)).is_empty());
    }

    #[test]
    fn folder_rows_are_ignored() {
        let mut f = StoryEntry::folder(PathBuf::from("/lib/sub"), "sub/");
        f.meta.ifid = "ZCODE-1".into();
        assert!(matching_entries(&[f], &record(&["ZCODE-1"], None)).is_empty());
    }

    #[test]
    fn the_walk_finds_a_story_below_the_root() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../zvm/tests/fixtures/etude.z5"))
            .expect("the zvm etude fixture");
        let root = crate::scratch_dir("library-match");
        let sub = root.join("infocom").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("etude.z5"), &bytes).unwrap();
        let ifid = crate::ifid::compute_ifid(&bytes);
        let data = crate::scratch_dir("library-match-data");
        let roots = DataRoots::single(data.clone());

        let held = library_holds(&root, &roots, &record(&[&ifid], None));
        assert_eq!(held, vec![sub.join("etude.z5")]);
        assert!(library_holds(&root, &roots, &record(&["ZCODE-0-000000-0000"], None)).is_empty());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&data);
    }

    fn dl(name: &str) -> DownloadOption {
        DownloadOption { filename: name.into(), url: format!("https://x/{name}"), format: None, title: None, desc: None }
    }

    #[test]
    fn an_option_is_found_by_name_in_a_subfolder() {
        let files = [PathBuf::from("/lib/glulx/a.ulx"), PathBuf::from("/lib/b.z5")];
        let got = option_paths(&[dl("a.ulx"), dl("c.z5")], &files);
        assert_eq!(got, vec![vec![PathBuf::from("/lib/glulx/a.ulx")], vec![]]);
    }

    #[test]
    fn option_names_compare_case_insensitively() {
        let files = [PathBuf::from("/lib/x/PHOTOPIA.Z5")];
        assert_eq!(option_paths(&[dl("Photopia.z5")], &files)[0], files.to_vec());
    }

    #[test]
    fn several_hits_come_back_sorted_and_deduplicated() {
        let files = [
            PathBuf::from("/lib/z/a.z5"),
            PathBuf::from("/lib/b/a.z5"),
            PathBuf::from("/lib/b/A.Z5"),
            PathBuf::from("/lib/b/a.z5"),
        ];
        assert_eq!(
            option_paths(&[dl("a.z5")], &files)[0],
            vec![PathBuf::from("/lib/b/A.Z5"), PathBuf::from("/lib/b/a.z5"), PathBuf::from("/lib/z/a.z5")]
        );
    }

    #[test]
    fn a_non_story_extension_matches_too() {
        let files = [PathBuf::from("/lib/zips/game.zip")];
        assert_eq!(option_paths(&[dl("game.zip")], &files)[0].len(), 1);
    }

    #[test]
    fn the_options_walk_finds_files_in_nested_folders_by_name_only() {
        let root = crate::scratch_dir("options-walk");
        let deep = root.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("Game.zip"), b"not a story").unwrap();
        std::fs::write(root.join("a").join("other.txt"), b"x").unwrap();
        let got = options_in_library(&root, &[dl("game.zip"), dl("missing.z5")]);
        assert_eq!(got, vec![vec![deep.join("Game.zip")], vec![]]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// With a finished index the IFID match comes from it alone: the entry's
    /// file does not exist on disk, so a hashing walk could not have found it.
    #[test]
    fn a_supplied_index_decides_held_without_reading_any_story() {
        let root = crate::scratch_dir("resolve-index");
        let data = crate::scratch_dir("resolve-index-data");
        let roots = DataRoots::single(data.clone());
        let ghost = root.join("ghost").join("g.z5");
        let idx = [entry(ghost.to_str().unwrap(), "ZCODE-1", None)];
        let (held, _) = resolve_in_library(&root, &roots, Some(&record(&["ZCODE-1"], None)), &[], Some(&idx));
        assert_eq!(held, vec![ghost]);
        let (held, _) = resolve_in_library(&root, &roots, Some(&record(&["ZCODE-1"], None)), &[], None);
        assert!(held.is_empty(), "without the index the walk finds nothing on an empty disk");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Benchmark-style, not a check: ~200 folders x 25 files, names only.
    #[test]
    #[ignore]
    fn options_walk_timing_over_five_thousand_files() {
        let root = crate::scratch_dir("options-bench");
        for d in 0..200 {
            let dir = root.join(format!("folder-{d}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..25 {
                std::fs::write(dir.join(format!("file-{f}.z5")), b"x").unwrap();
            }
        }
        let opts = [dl("file-3.z5"), dl("nope.z5")];
        let t = std::time::Instant::now();
        let got = options_in_library(&root, &opts);
        let took = t.elapsed();
        eprintln!("options_in_library over 200 folders x 25 files: {took:?}");
        assert_eq!(got[0].len(), 200);
        let _ = std::fs::remove_dir_all(&root);
    }
}
