//! Per-story metadata cache: `<data_base>/<story-key>.save/info.json`.
//!
//! Since SQ-1723 the fetched IFDB record and cover are not in that file but in
//! the catalogue's shared store ([`crate::ifdb_store`]), once per IFDB entry;
//! `info.json` keeps the copy's link to its entry (and the probe block), and
//! [`load`]/[`save`] assemble and split so callers still see one [`StoryInfo`].
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
    /// Filename of the cached cover: in the shared store ("<tuid>.jpg") when the
    /// record is linked, else the copy's own "cover.png". Derived on load from
    /// which file exists; never trusted from disk.
    pub cover: Option<String>,
    pub not_found: bool,
}

/// SQ-0276's slot. Defined here so writes preserve it; not populated by SQ-0348.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeMeta {
    pub probed_at: Option<String>,
}

/// A copy's own file: `<game_dir>/info.json`. Since SQ-1723 it holds the link
/// state, not the IFDB record (see [`StoryInfo`]).
pub fn info_path(game_dir: &Path) -> PathBuf { game_dir.join("info.json") }

/// A copy's link to a shared IFDB entry: which tuid it is linked to. The
/// record behind it lives in [`crate::ifdb_store`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Link {
    tuid: String,
}

/// What a copy's `info.json` holds on disk.
///
/// * `link` — the copy is linked to a shared IFDB entry (the normal found case).
/// * `fetched` — a record that cannot be shared and so stays with the copy: an
///   authoritative not-found, a curated row, or one with no usable tuid. A
///   `fetched` that *could* be shared is the pre-SQ-1723 shape; reading it
///   moves it into the store ([`adopt_if_legacy`]).
///
/// [`StoryInfo`] is the assembled view of this plus the shared record, so
/// callers see one struct, as they always did.
#[derive(Serialize, Deserialize)]
struct CopyInfo {
    format_version: u32,
    ifid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<Link>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fetched: Option<FetchedMeta>,
    #[serde(default)]
    probe: Option<ProbeMeta>,
}

fn read_copy(game_dir: &Path) -> Option<CopyInfo> {
    let raw = std::fs::read(info_path(game_dir)).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn write_copy(game_dir: &Path, copy: &CopyInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(game_dir)?;
    let json = serde_json::to_string_pretty(copy)?;
    // Atomic (SQ-1676): the catalogue is shared, so two players may refresh the
    // same story at once; a reader must see the old file or the new one.
    crate::storage::atomic_write(&info_path(game_dir), json.as_bytes())
}

/// The IFDB page id this copy is linked to, from its link or (not yet adopted)
/// its own record. No identity check: the id is the key to a shared entry, and
/// whatever it names is right for itself.
pub fn linked_tuid(game_dir: &Path) -> Option<String> {
    let copy = read_copy(game_dir)?;
    copy.link
        .map(|l| l.tuid)
        .or_else(|| copy.fetched.and_then(|f| f.ifdb_tuid))
        .filter(|t| crate::ifdb_store::is_valid_tuid(t))
}

/// Adoption (SQ-1723): if this copy still holds a shareable IFDB record of its
/// own (the layout before the shared store), move it and its `cover.png` into
/// the store under its tuid and leave only the link behind. The newer fetch
/// wins if the store already has the entry; nothing is refetched. Best-effort:
/// on any failure the copy is left as it was and is read as before.
pub(crate) fn adopt_if_legacy(game_dir: &Path) {
    let Some(mut copy) = read_copy(game_dir) else { return };
    let Some(tuid) = copy.fetched.as_ref().and_then(crate::ifdb_store::shared_tuid).map(str::to_string) else {
        return;
    };
    let Some(cat) = crate::ifdb_store::catalogue_of(game_dir) else { return };
    let legacy = copy.fetched.take().expect("matched above");
    if crate::ifdb_store::adopt_legacy(cat, game_dir, &tuid, &legacy, &info_path(game_dir)).is_ok() {
        copy.link = Some(Link { tuid });
        let _ = write_copy(game_dir, &copy);
    }
}

/// Load, or None if absent/unreadable/malformed/wrong-version/wrong-IFID.
/// Never an error: absent metadata is a normal state, not a failure.
///
/// A copy linked to a shared IFDB entry (SQ-1723) comes back with that entry's
/// record as `fetched`; a link whose entry is missing has no `fetched`, so it
/// reads as never fetched.
pub fn load(game_dir: &Path, expect_ifid: &str) -> Option<StoryInfo> {
    let copy = read_copy(game_dir)?;
    if copy.format_version != FORMAT_VERSION || copy.ifid != expect_ifid {
        return None;
    }
    adopt_if_legacy(game_dir);
    // Re-read: adoption rewrote it. A failed adoption leaves it as it was.
    let copy = read_copy(game_dir).unwrap_or(copy);
    let fetched = match &copy.link {
        Some(link) => crate::ifdb_store::catalogue_of(game_dir)
            .and_then(|cat| crate::ifdb_store::read_record(cat, &link.tuid)),
        None => copy.fetched,
    };
    Some(StoryInfo { format_version: copy.format_version, ifid: copy.ifid, fetched, probe: copy.probe })
}

/// Save: a shareable record goes to the shared store (unless the store holds a
/// strictly newer fetch of the same entry) and the copy keeps only its link;
/// anything else stays in the copy's own file. A save that carries no `fetched`
/// keeps the copy's existing link.
pub fn save(game_dir: &Path, info: &StoryInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(game_dir)?;
    let mut copy = CopyInfo {
        format_version: info.format_version,
        ifid: info.ifid.clone(),
        link: None,
        fetched: None,
        probe: info.probe.clone(),
    };
    match &info.fetched {
        Some(f) => match (crate::ifdb_store::shared_tuid(f), crate::ifdb_store::catalogue_of(game_dir)) {
            (Some(tuid), Some(cat)) => {
                crate::ifdb_store::write_record_unless_older(cat, tuid, f)?;
                copy.link = Some(Link { tuid: tuid.to_string() });
            }
            _ => copy.fetched = Some(f.clone()),
        },
        None => copy.link = read_copy(game_dir).and_then(|old| old.link),
    }
    write_copy(game_dir, &copy)
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
