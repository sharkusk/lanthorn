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

use std::path::{Path, PathBuf};

use crate::data_roots::DataRoots;
use crate::ifiction::IFiction;
use crate::picker::StoryEntry;

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
}
