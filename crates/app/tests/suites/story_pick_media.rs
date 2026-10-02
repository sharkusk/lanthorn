//! `--story <n|name>` against a real compilation disc (SQ-1078).
//!
//! The unit cases in `app::story_pick` prove the matching over rows a test made
//! up. This proves the half that made the flag worth building: that the rows are
//! the ones a real volume offers, and that naming one comes back with the pair
//! that opens it — the container's path plus WHICH story on it.
//!
//! The specimen is `stories/InfocomMasterpieces.img`, the Macintosh compilation
//! volume, whose own tiebreak opens `INFOCOMMASTERPIECES/ZORK ZERO/STORY.DATA`
//! and nothing else. Arthur is on that platter and was, until this flag,
//! unreachable from any command line — SQ-1063 measured his Macintosh press off
//! a StuffIt archive unpacked beside the disc instead, which is a directory
//! rather than a medium, so the profile resolved wrong and every number
//! described a screen no player sees.
//!
//! Fixtures are gitignored, so every case skips vacuously when the disc is
//! absent.

use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The disc, and what a launch argument makes of it — `None` when it is absent.
fn masterpieces() -> Option<(PathBuf, app::picker::StorySource, PathBuf)> {
    let path = stories_dir().join("InfocomMasterpieces.img");
    if !path.is_file() {
        eprintln!("SKIP: gitignored compilation volume absent");
        return None;
    }
    let data_base = std::env::temp_dir().join("lanthorn-sq1078-story-pick");
    let source = app::picker::StorySource::of(&path, &app::data_roots::DataRoots::single(&data_base))
        .expect("a disc holding several games is a source of stories");
    Some((path, source, data_base))
}

/// The whole point of the flag: a name reaches a game the mount does not prefer.
///
/// Both halves are asserted, because either alone is satisfiable by an accident:
/// the disc must really offer a choice (non-vacuity — a one-game floppy would
/// pass a "the name matched" assertion trivially), and the pick must come back
/// with Arthur's own `disk_entry` rather than the path, which would boot Zork
/// Zero exactly as it did before.
#[test]
fn a_name_reaches_the_game_the_discs_own_tiebreak_does_not() {
    let Some((path, source, data_base)) = masterpieces() else { return };

    // NON-VACUITY: a shelf, not a floppy — and one whose default is not Arthur.
    let rows = source.scan(&app::data_roots::DataRoots::single(&data_base));
    assert!(rows.len() > 2, "a compilation offering a choice: {} rows", rows.len());

    let (chosen, entry) = app::story_pick::pick(Some(&source), &path, &app::data_roots::DataRoots::single(&data_base), "arthur")
        .expect("Arthur is on this platter");
    assert_eq!(chosen, path, "the CONTAINER is what gets opened");
    let entry = entry.expect("and which story on it — the thing that reaches the right game");
    assert!(
        entry.to_ascii_uppercase().contains("ARTHUR"),
        "Arthur's own entry, not the tiebreak's: {entry}"
    );

    // The tiebreak, for contrast: this is what a launch WITHOUT the flag gets,
    // and it is a different game.
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("readable")).expect("mounts");
    let (default, _) = hfs.story().expect("the disc carries a game");
    assert_ne!(default, entry, "the flag reached past the volume's own preference");

    // And a number says the same thing as the name it stands beside: the rows
    // are the browser's, in the browser's order.
    let i = 1 + rows.iter().position(|r| r.meta.disk_entry.as_deref() == Some(&entry)).unwrap();
    let (by_number, entry_by_number) =
        app::story_pick::pick(Some(&source), &path, &app::data_roots::DataRoots::single(&data_base), &i.to_string()).expect("in range");
    assert_eq!((by_number, entry_by_number.as_deref()), (path, Some(entry.as_str())));
}

/// A miss refuses with the list. It must never fall back to booting whatever the
/// mount preferred — that failure is silent, self-consistent, and indisting-
/// uishable from a working flag until somebody reads the frame.
#[test]
fn a_name_no_game_on_the_disc_answers_to_is_refused_with_the_menu() {
    let Some((path, source, data_base)) = masterpieces() else { return };

    let err = app::story_pick::pick(Some(&source), &path, &app::data_roots::DataRoots::single(&data_base), "photopia")
        .expect_err("Photopia is not an Infocom game");
    assert!(err.starts_with("no story on this disk is named 'photopia':"), "{err}");
    assert!(err.to_ascii_uppercase().contains("ZORK ZERO"), "the menu rides along: {err}");

    // Out of range says the range rather than clamping to an end of the list.
    let err = app::story_pick::pick(Some(&source), &path, &app::data_roots::DataRoots::single(&data_base), "999")
        .expect_err("no 999th game anywhere");
    assert!(err.starts_with("no story 999 on this disk — pick 1 to "), "{err}");
}

/// `STORY.DATA` is the name of nearly every game on a Macintosh compilation, so
/// it is exactly the spelling that must refuse rather than pick the first one.
#[test]
fn a_name_several_games_share_refuses_rather_than_guessing() {
    let Some((path, source, data_base)) = masterpieces() else { return };

    let err = app::story_pick::pick(Some(&source), &path, &app::data_roots::DataRoots::single(&data_base), "story.data")
        .expect_err("this disc stores several games under that name");
    assert!(err.starts_with("'story.data' matches more than one story on this disk:"), "{err}");
}

/// A plain story file offers one row, and the flag is matched against it like
/// any other list — the rule is "match what this path offers", so a script
/// sweeping mixed media need not know which of its arguments are compilations.
/// A wrong name still says so instead of booting the file anyway.
#[test]
fn a_lone_story_file_is_a_list_of_one() {
    let path = stories_dir().join("zork0-r393-s890714.z6");
    if !path.is_file() {
        eprintln!("SKIP: gitignored story absent");
        return;
    }
    let data_base = std::env::temp_dir().join("lanthorn-sq1078-story-pick-lone");
    // Not a source of stories: there is no choice on a single file (SQ-0844).
    assert!(app::picker::StorySource::of(&path, &app::data_roots::DataRoots::single(&data_base)).is_none());

    let (chosen, entry) = app::story_pick::pick(None, &path, &app::data_roots::DataRoots::single(&data_base), "1").expect("the one row");
    assert_eq!(chosen, path);
    assert_eq!(entry, None, "a loose file has no entry to name");

    let err = app::story_pick::pick(None, &path, &app::data_roots::DataRoots::single(&data_base), "arthur")
        .expect_err("this file is not Arthur");
    assert!(err.starts_with("no story on this file is named 'arthur':"), "{err}");
}
