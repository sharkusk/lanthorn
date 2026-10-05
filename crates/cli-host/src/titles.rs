//! Canonical game titles, keyed by the build rather than by a filename.
//!
//! The table lived in `app::session` until SQ-0850, when the save-directory key
//! needed it too: a story mounted out of a disk image is named by its *release
//! and serial*, and the readable half of that name is the title this table
//! gives. `app` and `zvm-cli` must agree on the directory a game's saves live
//! in, so the lookup they both read from is here, beside [`crate::storage`],
//! rather than in either front-end.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Canonical titles for well-known games, keyed by the release+serial prefix of
/// the IFID (`ZCODE-<release>-<serial>`, WITHOUT the trailing byte-checksum),
/// bundled in `known_titles.tsv` (`include_str!`d at build time). The key is
/// robust to different file copies of the same release. Used to prefer a clean
/// canonical name over the opening-banner heuristic, by the story picker, and by
/// the per-game save-directory key.
fn known_titles() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("known_titles.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                line.split_once('\t').map(|(k, v)| (k.trim(), v.trim()))
            })
            .collect()
    })
}

/// The canonical title for a known game, matched on the release+serial prefix of
/// the IFID (the trailing `-<checksum>` is ignored).
pub fn known_title(ifid: &str) -> Option<&'static str> {
    // Strip the trailing checksum segment: "ZCODE-88-840726-A129" → "ZCODE-88-840726".
    let key = ifid.rsplit_once('-').map_or(ifid, |(prefix, _)| prefix);
    known_titles().get(key).copied()
}

/// The canonical title for a Z-code build, straight from its header release and
/// serial — the same lookup as [`known_title`] without an IFID to build first.
pub fn title_for_build(release: u16, serial: &str) -> Option<&'static str> {
    known_titles().get(format!("ZCODE-{release}-{serial}").as_str()).copied()
}

/// Canonical titles for the Infocom games, keyed by IFDB tuid, bundled in
/// `infocom_tuids.tsv`. Titles are spelled as `known_titles.tsv` spells them.
fn infocom_tuids() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("infocom_tuids.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                line.split_once('\t').map(|(k, v)| (k.trim(), v.trim()))
            })
            .collect()
    })
}

/// The canonical Infocom title for an IFDB tuid, or `None` for a game that is
/// not one of the Infocom titles in `infocom_tuids.tsv`.
pub fn title_for_tuid(tuid: &str) -> Option<&'static str> {
    infocom_tuids().get(tuid).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_title_looks_up_table() {
        assert_eq!(known_title("ZCODE-116-870602-FC65"), Some("Bureaucracy"));
        assert_eq!(known_title("ZCODE-77-850814-5031"), Some("A Mind Forever Voyaging"));
        assert_eq!(known_title("ZCODE-0-000000-0000"), None);
        // Entries added when the table moved to the bundled known_titles.tsv.
        assert_eq!(known_title("ZCODE-27-831005-X"), Some("Deadline"));
        assert_eq!(known_title("ZCODE-48-840904-X"), Some("Zork II: The Wizard of Frobozz"));
        assert_eq!(known_title("ZCODE-29-860820-X"), Some("Enchanter"));
        // Alternate releases (not the copy we own) resolve from the full catalog.
        assert_eq!(known_title("ZCODE-23-820428-X"), Some("Zork I: The Great Underground Empire"));
        assert_eq!(known_title("ZCODE-15-840612-X"), Some("Seastalker"));
        // v6 reference entries resolve even though lanthorn can't launch them yet.
        assert_eq!(known_title("ZCODE-296-881019-X"), Some("Zork Zero: The Revenge of Megaboz"));
    }

    #[test]
    fn known_titles_file_parses_without_dupes() {
        let table = known_titles();
        assert!(table.len() >= 30, "bundled table has the verified entries: {}", table.len());
        // Keys are unique IFID prefixes (HashMap would silently dedupe; assert the
        // line count matches the entry count so a duplicate prefix is caught).
        let lines = include_str!("known_titles.tsv")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert_eq!(lines, table.len(), "no duplicate IFID prefixes in known_titles.tsv");
    }

    /// The build-keyed door is the IFID-keyed one with the checksum never
    /// invented in the first place — the save-directory key has a header in hand
    /// and no reason to build an IFID out of it.
    #[test]
    fn title_for_build_matches_the_ifid_lookup() {
        assert_eq!(title_for_build(88, "840726"), known_title("ZCODE-88-840726-X"));
        assert_eq!(title_for_build(59, "851108"), Some("The Hitchhiker's Guide to the Galaxy"));
        assert_eq!(title_for_build(0, "000000"), None);
    }

    /// Every tuid row names a title `known_titles.tsv` also carries, and no tuid
    /// is listed twice.
    #[test]
    fn infocom_tuids_file_names_known_titles_without_dupes() {
        let table = infocom_tuids();
        let lines = include_str!("infocom_tuids.tsv")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert_eq!(lines, table.len(), "no duplicate tuids in infocom_tuids.tsv");
        let known: Vec<&str> = known_titles().values().copied().collect();
        for (tuid, title) in table {
            assert!(known.contains(title), "{tuid}: {title} is not spelled as in known_titles.tsv");
        }
        assert_eq!(title_for_tuid("0dbnusxunq7fw5ro"), Some("Zork I: The Great Underground Empire"));
    }
}
