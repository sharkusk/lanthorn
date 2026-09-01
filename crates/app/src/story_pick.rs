//! `--story <n|name>`: naming a game on a volume that holds several, without
//! the browser (SQ-1078).
//!
//! A compilation disc has exactly one door today, and a person has to walk
//! through it: launch the image, read the list, move the cursor. That is fine
//! for a player and useless for everything else — no harness, no capture and no
//! bug report can reach any story on `InfocomMasterpieces.img` except whichever
//! one the format's own tiebreak happens to prefer. SQ-1063 measured the
//! Macintosh *Arthur* press off a StuffIt archive unpacked beside the disc for
//! exactly this reason, and a loose directory is not a medium, so it booted
//! under the wrong interpreter profile and the numbers described a screen no
//! player sees.
//!
//! **The matching rule is not here.** It is `cli_host::story_pick::find`, shared
//! with `zvm-cli`, which has offered this flag since SQ-0834 — a flag spelled
//! the same in both front-ends that *matched* differently would be its own
//! defect. What is here is the lanthorn half: which rows a launch argument
//! offers, and how a [`StoryEntry`] reads as a menu line.

use std::path::{Path, PathBuf};

use crate::picker::{StoryEntry, StorySource};

/// How one browser row reads to the chooser.
///
/// `name` is what the volume stores the story under — the same
/// `LEATHRGODDESSES` / `ARTHUR FOLDER/STORY.DATA` string that
/// [`crate::picker::StoryMeta::disk_entry`] carries and that the launch hands
/// back to the mount, because that is the only thing telling two rows off one
/// disc apart. A loose story file has no such name, so its filename stands in.
pub fn row_of(entry: &StoryEntry) -> cli_host::story_pick::Row {
    let name = entry.meta.disk_entry.clone().unwrap_or_else(|| entry.filename.clone());
    cli_host::story_pick::Row { label: label_of(entry, &name), name, title: Some(entry.title.clone()) }
}

/// The line a menu prints for this row: the title it goes by, the build behind
/// it, and — when they differ — the name it came off the medium under.
///
/// The stored name earns its place the same way it does in `zvm-cli`'s menu:
/// *Masterpieces* carries *Ballyhoo* three times, one build under three
/// filenames, and three identical lines are not a choice anybody can make.
fn label_of(entry: &StoryEntry, name: &str) -> String {
    let mut s = entry.title.clone();
    if let (Some(v), Some(r), Some(serial)) =
        (entry.meta.version.as_deref(), entry.meta.release, entry.meta.serial.as_deref())
    {
        s.push_str(&format!("  (v{v} r{r} s{serial})"));
    }
    if name != entry.title {
        s.push_str(&format!("  {name}"));
    }
    s
}

/// The noun the error text calls what was searched.
///
/// A `DiskSet` of one zip is not a disk (SQ-1098) — the variant means "the
/// stories this argument offers", and since a zip holding two games became one
/// of them the noun has to be read off the members rather than off the variant.
/// The four-byte sniff runs only on the refusal path, where a message that
/// calls a download a disk is the whole of what is being fixed.
fn subject_of(source: Option<&StorySource>) -> &'static str {
    match source {
        Some(StorySource::Library(_)) => "this library",
        Some(StorySource::DiskSet { members, .. })
            if !members.is_empty() && members.iter().all(|m| crate::hints::is_zip(m)) =>
        {
            "this archive"
        }
        Some(StorySource::DiskSet { .. }) => "this disk",
        // No source at all: the launch argument is one story file, and the only
        // list there is is the one story it holds.
        None => "this file",
    }
}

/// Which story `--story <want>` asked for, out of the rows this launch offers.
///
/// Returns what a launch needs and nothing else: the container's path, and
/// **which** story on it — the pair [`StoryEntry::is`] treats as an identity,
/// because the path alone stopped being one the moment a single image could
/// contribute thirty-three rows.
///
/// `source` is what [`StorySource::of`] made of the launch argument, so the rows
/// are exactly the ones the browser would have shown. `None` means the argument
/// is an ordinary story file or a one-game disk, and `--story` is matched
/// against the single row it offers rather than refused: the rule is "match what
/// this path offers", however many that is, so a script that passes the flag
/// over a shelf of mixed media does not have to know which entries are
/// compilations. A name that does not match still says so instead of booting
/// something else, which is the property that matters.
pub fn pick(
    source: Option<&StorySource>,
    single_file: &Path,
    data_base: &Path,
    want: &str,
) -> Result<(PathBuf, Option<String>), String> {
    let entries = match source {
        Some(source) => source.scan(data_base),
        None => crate::picker::resolve_entries(single_file, data_base),
    };
    if entries.is_empty() {
        return Err(format!("no story to open on {}", single_file.display()));
    }
    let i = resolve(&entries, want, subject_of(source))?;
    let chosen = &entries[i];
    Ok((chosen.path.clone(), chosen.meta.disk_entry.clone()))
}

/// The same choice, for the headless instruments: which story on `path` a
/// `--entry <n|name>` spec names, ready to hand to
/// [`crate::hints::load_mounted_story_from`] and [`crate::graphics::PictSource::resolve`].
///
/// `Ok(None)` is "this path is one story", which is what a loose file and a
/// one-game floppy both answer; those callers pass it straight through, exactly
/// as they passed `None` before.
///
/// The examples used to take the stored name **literally**, which meant knowing
/// `InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA` before you could measure
/// anything on that disc — and the only way to learn it was to mount the disc.
/// One rule for the flag and the instruments means `--entry arthur` works
/// wherever `--story arthur` does, and a fragment that fits two games is refused
/// with the list rather than resolved to the first.
pub fn entry_on(path: &Path, want: &str) -> Result<Option<String>, String> {
    // Metadata cache only: the scan reads fetched sidecars from here and is
    // content with a directory that does not exist. Nothing is written.
    let data_base = std::env::temp_dir().join("lanthorn-instrument-scan");
    let source = StorySource::of(path, &data_base);
    Ok(pick(source.as_ref(), path, &data_base, want)?.1)
}

/// The shared rule, over browser rows: a 1-based number, or a case-insensitive
/// substring of the stored name or the title, refusing rather than guessing when
/// it picks out more than one.
pub fn resolve(entries: &[StoryEntry], want: &str, subject: &str) -> Result<usize, String> {
    let rows: Vec<cli_host::story_pick::Row> = entries.iter().map(row_of).collect();
    cli_host::story_pick::find(&rows, want, subject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::{Engine, Features, RowKind, StoryMeta};

    fn entry(path: &str, title: &str, disk_entry: Option<&str>, release: u16) -> StoryEntry {
        let filename =
            Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned();
        StoryEntry {
            path: PathBuf::from(path),
            title: title.into(),
            filename,
            meta: StoryMeta {
                size_bytes: 1,
                story_bytes: 1,
                modified: None,
                engine: Engine::ZCode,
                format: "Z-code".into(),
                version: Some("6".into()),
                serial: Some("890606".into()),
                release: Some(release),
                ifid: format!("IFID-{release}"),
                features: Features::default(),
                self_blorb: None,
                disk_image: None,
                disk_entry: disk_entry.map(str::to_string),
                author: None,
                year: None,
                genre: None,
                language: None,
                description: None,
                ifdb_link: None,
                ifdb_rating: None,
                ifdb_rating_count: None,
                fetch_not_found: false,
            },
            hint_sidecar: None,
            kind: RowKind::Story,
        }
    }

    fn disc() -> Vec<StoryEntry> {
        vec![
            entry(
                "InfocomMasterpieces.img",
                "Arthur: The Quest for Excalibur",
                Some("MAC/ARTHUR FOLDER/STORY.DATA"),
                54,
            ),
            entry("InfocomMasterpieces.img", "Ballyhoo", Some("MAC/BALLYHOO"), 97),
            entry(
                "InfocomMasterpieces.img",
                "Zork Zero: The Revenge of Megaboz",
                Some("PC/DATA/ZORK0.DAT"),
                296,
            ),
        ]
    }

    /// The point of the whole flag: a name reaches the game it names, and what
    /// comes back is the pair that opens it — the CONTAINER's path, plus which
    /// story on it. The path alone would open Arthur's disc and boot whatever
    /// the mount preferred, which is the state SQ-1078 exists to leave.
    #[test]
    fn a_name_picks_that_storys_entry_off_a_multi_story_volume() {
        let d = disc();
        let i = resolve(&d, "arthur", "this disk").expect("one Arthur");
        assert_eq!(d[i].meta.disk_entry.as_deref(), Some("MAC/ARTHUR FOLDER/STORY.DATA"));
        // …and by the title the browser shows, not only the stored name.
        assert_eq!(resolve(&d, "Megaboz", "this disk"), Ok(2));
        // …and by position in the list the browser would have shown.
        assert_eq!(resolve(&d, "2", "this disk"), Ok(1));
    }

    /// A miss must never fall back to booting an arbitrary game — the failure
    /// mode is silent and self-consistent, and the menu comes with the refusal
    /// so the next attempt is informed.
    #[test]
    fn a_name_that_matches_nothing_refuses_and_shows_the_list() {
        let err = resolve(&disc(), "trinity", "this disk").unwrap_err();
        assert!(err.starts_with("no story on this disk is named 'trinity':"), "{err}");
        assert!(err.contains("Ballyhoo"), "the menu rides along: {err}");
        let err = resolve(&disc(), "9", "this disk").unwrap_err();
        assert!(err.starts_with("no story 9 on this disk — pick 1 to 3:"), "{err}");
    }

    #[test]
    fn an_ambiguous_name_refuses_rather_than_guessing() {
        let err = resolve(&disc(), "MAC/", "this disk").unwrap_err();
        assert!(err.starts_with("'MAC/' matches more than one story on this disk:"), "{err}");
    }

    /// The label is what makes a refusal usable, so it names the build and keeps
    /// the stored name beside the title: one disc carries *Ballyhoo* three times
    /// under three filenames, and three identical lines are not a choice.
    #[test]
    fn a_row_reads_as_its_title_its_build_and_the_name_the_medium_stored() {
        let row = row_of(&disc()[1]);
        assert_eq!(row.label, "Ballyhoo  (v6 r97 s890606)  MAC/BALLYHOO");
        assert_eq!(row.name, "MAC/BALLYHOO");
        assert_eq!(row.title.as_deref(), Some("Ballyhoo"));
        // A loose story file has no stored name, so its filename stands in —
        // and is not repeated when it is already the title.
        let loose = entry("stories/zork0.z6", "zork0.z6", None, 393);
        let row = row_of(&loose);
        assert_eq!(row.name, "zork0.z6");
        assert_eq!(row.label, "zork0.z6  (v6 r393 s890606)");
    }

    /// The subject is the noun the message uses, and it comes from what the
    /// launch argument turned out to be — a directory of stories is not a disk.
    #[test]
    fn the_subject_names_what_was_searched() {
        assert_eq!(subject_of(None), "this file");
        assert_eq!(subject_of(Some(&StorySource::Library(PathBuf::from("stories")))), "this library");
        assert_eq!(
            subject_of(Some(&StorySource::DiskSet {
                dir: PathBuf::from("stories"),
                members: vec![PathBuf::from("stories/a.img")],
            })),
            "this disk"
        );
    }
}
