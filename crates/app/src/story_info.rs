//! Per-story metadata cache: `<data_base>/<story-key>.save/info.json`.
//!
//! Caches ONLY what cannot be cheaply recomputed from the story file — the IFDB
//! fetch, and (SQ-0276) a runtime capability probe. A blorb's own `IFmd` is NOT
//! cached: `scan_stories` already holds the bytes, so the blorb is the cache.
//!
//! Keyed by filename (SQ-0284) but describing an IFID, so `load` checks the two
//! agree — see `load`.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;

/// The fetch algorithm's version. **Bump when a re-fetch would produce a
/// materially different block** — a new field extracted, a changed endpoint,
/// fixed parsing. Do NOT bump for refactors that cannot change output, and do
/// NOT tie this to CARGO_PKG_VERSION: that would re-fetch every story in every
/// library on every release, for nothing.
///
/// `r` skips stories already fetched at this version, which is what makes it
/// double as the rescan-all: bump this and the next `r` refreshes the library.
pub const FETCH_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryInfo {
    pub format_version: u32,
    /// The IFID these blocks describe. Checked against the story on disk.
    pub ifid: String,
    pub fetched: Option<FetchedMeta>,
    /// Reserved for SQ-0276. Always None here, but preserved across writes.
    pub probe: Option<ProbeMeta>,
}

/// Present ONLY for a fetch that ran to completion — found, or authoritatively
/// not-found. A transport error writes no block, so `r` retries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchedMeta {
    pub scanned_at: String,
    pub fetch_version: u32,
    pub source: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub ifdb_tuid: Option<String>,
    pub ifdb_link: Option<String>,
    /// IFDB's community average rating, 1–5 (SQ-0529). `None` for an unrated
    /// game — never `0.0`, which is a rating the list would have to show.
    pub ifdb_rating: Option<f32>,
    /// The number of ratings behind `ifdb_rating`; the rating sort's tiebreak.
    pub ifdb_rating_count: Option<u32>,
    /// Filename of the cached cover beside this file, e.g. "cover.png".
    pub cover: Option<String>,
    pub not_found: bool,
}

/// SQ-0276's slot. Defined here so writes preserve it; not populated by SQ-0348.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeMeta {
    pub probed_at: Option<String>,
}

pub fn info_path(game_dir: &Path) -> PathBuf { game_dir.join("info.json") }

/// Load, or None if absent/unreadable/malformed/wrong-version/wrong-IFID.
/// Never an error: absent metadata is a normal state, not a failure.
pub fn load(game_dir: &Path, expect_ifid: &str) -> Option<StoryInfo> {
    let raw = std::fs::read(info_path(game_dir)).ok()?;
    let info: StoryInfo = serde_json::from_slice(&raw).ok()?;
    if info.format_version != FORMAT_VERSION || info.ifid != expect_ifid {
        return None;
    }
    Some(info)
}

pub fn save(game_dir: &Path, info: &StoryInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(game_dir)?;
    let json = serde_json::to_string_pretty(info)?;
    std::fs::write(info_path(game_dir), json)
}

/// The `r`/`f` skip decision. `forced` (`f`) ignores the cache entirely.
pub fn needs_fetch(info: Option<&StoryInfo>, forced: bool) -> bool {
    if forced {
        return true;
    }
    match info.and_then(|i| i.fetched.as_ref()) {
        Some(f) => f.fetch_version != FETCH_VERSION,
        None => true,
    }
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique temp dir per call, safe under parallel test execution: process
    /// id plus a monotonic counter (the pointer-address trick from the brief's
    /// sketch can collide when the stack slot is reused across threads).
    fn tmp() -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("bm_story_info_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fetched(v: u32) -> FetchedMeta {
        FetchedMeta {
            scanned_at: "2026-07-16T00:00:00Z".into(),
            fetch_version: v,
            source: "ifdb".into(),
            title: Some("Zork I".into()),
            author: Some("Marc Blank and Dave Lebling".into()),
            language: None, first_published: Some("1980".into()), genre: None,
            description: None, ifdb_tuid: None, ifdb_link: None,
            ifdb_rating: Some(3.8), ifdb_rating_count: Some(226), cover: None,
            not_found: false,
        }
    }

    fn info(ifid: &str, f: Option<FetchedMeta>) -> StoryInfo {
        StoryInfo { format_version: FORMAT_VERSION, ifid: ifid.into(), fetched: f, probe: None }
    }

    #[test]
    fn round_trips() {
        let d = tmp();
        let i = info("ZCODE-52-871125", Some(fetched(FETCH_VERSION)));
        save(&d, &i).unwrap();
        assert_eq!(load(&d, "ZCODE-52-871125"), Some(i));
    }

    /// SPEC "Identity check": the sidecar is keyed by FILENAME but describes an
    /// IFID. Swap a different game in under the same filename and the stale
    /// sidecar would otherwise hand it Zork's blurb and cover.
    #[test]
    fn a_sidecar_for_a_different_ifid_is_ignored_entirely() {
        let d = tmp();
        save(&d, &info("ZCODE-52-871125", Some(fetched(FETCH_VERSION)))).unwrap();
        assert_eq!(load(&d, "ZCODE-88-840726"), None, "wrong IFID → every block stale");
    }

    #[test]
    fn unknown_format_version_is_ignored_not_an_error() {
        let d = tmp();
        let mut i = info("X", None);
        i.format_version = 9999;
        save(&d, &i).unwrap();
        assert_eq!(load(&d, "X"), None);
    }

    #[test]
    fn malformed_json_is_ignored() {
        let d = tmp();
        std::fs::write(info_path(&d), b"{ not json").unwrap();
        assert_eq!(load(&d, "X"), None);
    }

    #[test]
    fn a_missing_sidecar_is_none_not_an_error() {
        assert_eq!(load(&tmp().join("nope"), "X"), None);
    }

    /// SPEC "The scan UI" — the skip table. This predicate is the quest's most
    /// breakable logic and costs nothing to test.
    #[test]
    fn needs_fetch_matches_the_spec_table() {
        // r (forced = false)
        assert!(needs_fetch(None, false), "never tried, or last attempt errored → fetch");
        assert!(needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION - 1)))), false), "older fetch_version → fetch");
        assert!(!needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION)))), false), "current, found → skip");
        let mut nf = fetched(FETCH_VERSION);
        nf.not_found = true;
        assert!(!needs_fetch(Some(&info("X", Some(nf))), false), "current, not_found → skip: a completed answer");
        // A sidecar that exists only for a probe block has never been fetched.
        assert!(needs_fetch(Some(&info("X", None)), false), "no fetched block → fetch");
        // f (forced = true) ignores all of it.
        assert!(needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION)))), true), "forced overrides current+found");
        assert!(needs_fetch(None, true));
    }

    /// A probe block must survive a fetch rewriting the fetched block — the two
    /// writers must not clobber each other (SQ-0276 depends on this).
    #[test]
    fn writing_a_fetched_block_preserves_an_existing_probe_block() {
        let d = tmp();
        let mut i = info("X", None);
        i.probe = Some(ProbeMeta::default());
        save(&d, &i).unwrap();
        let mut loaded = load(&d, "X").unwrap();
        loaded.fetched = Some(fetched(FETCH_VERSION));
        save(&d, &loaded).unwrap();
        let back = load(&d, "X").unwrap();
        assert!(back.probe.is_some() && back.fetched.is_some());
    }
}
