//! SQ-1098: a zip holding two stories, from the save key up to `--story`.
//!
//! SQ-1085 made a zip a VOLUME — entries classified by content, so an archive
//! carries any format lanthorn runs — and stopped one step short on purpose. A
//! zip holding two games played the first one silently, with no way to ask for
//! the other, because the per-game save key had no per-entry form: `DiskBuild`
//! is defined in terms of a `blorb::medium::DiskImage` a zip does not have, so
//! `story_key_for` fell through to the ARCHIVE's basename and both games would
//! have shared one `default.lanthorn`. Enumerating on top of that would have
//! traded a visible limitation for an invisible data defect.
//!
//! So the key comes first here too. The order of the cases is the order the work
//! had to happen in, and the first one is the falsifying one: it fails on the
//! old key, with both games naming the same directory.
//!
//! **Every fixture is built, never read.** `stories/` is gitignored, and none of
//! this needs a commercial press: what is under test is the container, and two
//! synthetic v5 images with different releases tell the two rows apart exactly
//! as two real games would. Nothing here skips.

use std::io::Write as _;
use std::path::{Path, PathBuf};

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A minimal-but-coherent v5 story image, distinguished by `release`.
///
/// Every clause of the loader's own predicate is satisfied, because
/// `hints::extract_story` is what decides whether a zip entry is a story and
/// `zvm::memory::Memory::new` is what decides whether the row is launchable —
/// a fixture that fails either is not testing the container at all.
fn zcode_v5(release: u16, serial: &[u8; 6]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 5;
    b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
    b[0x04..0x06].copy_from_slice(&96u16.to_be_bytes()); // high memory
    b[0x08..0x0A].copy_from_slice(&100u16.to_be_bytes()); // dictionary (static)
    b[0x0A..0x0C].copy_from_slice(&64u16.to_be_bytes()); // objects (dynamic)
    b[0x0C..0x0E].copy_from_slice(&70u16.to_be_bytes()); // globals (dynamic)
    b[0x0E..0x10].copy_from_slice(&96u16.to_be_bytes()); // static memory base
    b[0x12..0x18].copy_from_slice(serial);
    b[0x1A..0x1C].copy_from_slice(&(128u16 / 4).to_be_bytes()); // file length / 4
    b
}

/// The two games this suite packs. Different releases and serials, so a row, a
/// key and a loaded image can each be traced back to the game it came from.
fn amber() -> Vec<u8> {
    zcode_v5(41, b"890101")
}

fn beacon() -> Vec<u8> {
    zcode_v5(77, b"911231")
}

/// Write a zip at `path` holding each `(entry name, bytes)` in archive order,
/// STORED so what comes back out is byte-for-byte what went in.
fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
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

/// A scratch directory unique per CALL, not merely per process (SQ-1131): these
/// cases write files and delete their own tree, and a pid-keyed helper hands
/// every caller in a `cargo test` binary the same directory.
fn scratch(tag: &str) -> PathBuf {
    app::scratch_dir(tag)
}

/// The archive both halves of this suite are about: two games, one download.
fn two_game_zip(dir: &Path) -> PathBuf {
    let zip = dir.join("if-archive-pack.zip");
    write_zip(&zip, &[("amber.z5", amber()), ("beacon.z5", beacon())]);
    zip
}

// ── The save key, first ──────────────────────────────────────────────────────

/// **The falsifying case.** Two entries of one zip must resolve to two save
/// directories, and on the old key they resolved to one — the archive's own
/// basename, for both of them.
///
/// Asserted on the DIRECTORY rather than the token, because the token is not
/// what overwrites a save, and asserted through the row (`StoryEntry::game_dir`)
/// rather than by calling the rule directly, because the row is what the picker,
/// the badges, the info panel and the launch all key from.
#[test]
fn two_stories_in_one_zip_get_two_save_directories() {
    let dir = scratch("sq1098-key");
    let data = scratch("sq1098-key-data");
    let zip = two_game_zip(&dir);

    let rows = app::picker::resolve_entries(&zip, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 2, "one row per game in the archive: {rows:?}");

    let dirs: Vec<PathBuf> = rows.iter().map(|r| r.game_dir(&app::data_roots::DataRoots::single(&data))).collect();
    assert_ne!(dirs[0], dirs[1], "one zip, one save directory, was the defect");

    // …and neither of them is the ARCHIVE's directory, which is what they shared.
    let archive_dir = app::storage::game_dir(
        &data,
        &app::storage::story_key_for(app::storage::StoryOrigin {
            path: &zip,
            entry: None,
            build: None,
        }),
    );
    for d in &dirs {
        assert_ne!(*d, archive_dir, "a row must not key on the archive's own name");
    }

    // The key is the entry's basename — the same token the same game takes
    // loose, which is the module's existing promise applied one level in.
    let keys: Vec<String> = rows.iter().map(|r| r.story_key()).collect();
    assert!(keys.contains(&"amber.z5".to_string()), "{keys:?}");
    assert!(keys.contains(&"beacon.z5".to_string()), "{keys:?}");

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The key a row shows and the key the LAUNCH arrives at must be the same one.
///
/// The launch does not have the row in hand — it has the container's path and
/// the selector — so it reaches the key by a different door
/// (`storage::story_key_at_from`). Two doors to one directory is the property;
/// it is what `disk_story_rows` pins for a disc and what this pins for a zip.
#[test]
fn the_launch_and_the_list_key_a_zipped_story_alike() {
    let dir = scratch("sq1098-doors");
    let data = scratch("sq1098-doors-data");
    let zip = two_game_zip(&dir);

    for row in app::picker::resolve_entries(&zip, &app::data_roots::DataRoots::single(&data)) {
        let entry = row.meta.disk_entry.as_deref();
        assert!(entry.is_some(), "a row off a two-game archive names its entry");
        assert_eq!(
            app::storage::story_key_at_from(&zip, entry),
            row.story_key(),
            "{}: keys differently at launch than in the list",
            row.title,
        );
    }

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// **No regression for the common path.** A loose story file keys on its
/// basename and nothing else, which is a promise rather than an implementation
/// detail — every save anybody already has sits in a directory named that way.
///
/// The same bytes packed into a single-story zip still key on the ARCHIVE, since
/// there is no choice to make and no selector on the row.
#[test]
fn a_loose_storys_key_is_unchanged_and_a_lone_zip_keys_on_itself() {
    let dir = scratch("sq1098-loose");
    let data = scratch("sq1098-loose-data");

    let loose = dir.join("amber.z5");
    std::fs::write(&loose, amber()).unwrap();
    let rows = app::picker::resolve_entries(&loose, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].meta.disk_entry, None, "a loose file has no entry to name");
    assert_eq!(rows[0].story_key(), "amber.z5");
    assert_eq!(rows[0].game_dir(&app::data_roots::DataRoots::single(&data)), data.join("amber.z5.save"));

    let lone = dir.join("solo.zip");
    write_zip(&lone, &[("amber.z5", amber())]);
    let rows = app::picker::resolve_entries(&lone, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 1, "a one-game archive is one row, with no selector");
    assert_eq!(rows[0].meta.disk_entry, None);
    assert_eq!(rows[0].story_key(), "solo.zip");

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

// ── Then enumeration ─────────────────────────────────────────────────────────

/// The defect itself: both games are listed, where one of them used to be
/// unreachable however long you looked at the list.
///
/// The rows carry the pair that opens a game — the container's path plus WHICH
/// entry — because the path alone stopped being an identity the moment one file
/// could contribute two rows.
#[test]
fn both_stories_in_a_zip_are_listed_as_their_own_rows() {
    let dir = scratch("sq1098-list");
    let data = scratch("sq1098-list-data");
    let zip = two_game_zip(&dir);

    let rows = app::picker::resolve_entries(&zip, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 2, "{rows:?}");
    let mut entries: Vec<&str> =
        rows.iter().filter_map(|r| r.meta.disk_entry.as_deref()).collect();
    entries.sort();
    assert_eq!(entries, vec!["amber.z5", "beacon.z5"]);
    for r in &rows {
        assert_eq!(r.path, zip, "every row names the container it came out of");
    }
    // Each row is its own game, read off its own header rather than the
    // archive's tiebreak.
    let mut releases: Vec<u16> = rows.iter().filter_map(|r| r.meta.release).collect();
    releases.sort();
    assert_eq!(releases, vec![41, 77], "two builds, not one read twice");

    // And a library scan lists them too — the browser's own door.
    let rows = app::picker::scan_stories(&dir, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 2, "a zip in a library contributes a row per game");

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A zip holding two games is a SOURCE of stories, which is what opens the
/// browser instead of booting whichever entry comes first.
///
/// A zip holding one is not, so an ordinary download takes the single-file path
/// it always did with no picker friction at all.
#[test]
fn a_two_game_zip_is_a_source_and_a_one_game_zip_is_not() {
    let dir = scratch("sq1098-source");
    let data = scratch("sq1098-source-data");

    let zip = two_game_zip(&dir);
    let source = app::picker::StorySource::of(&zip, &app::data_roots::DataRoots::single(&data))
        .expect("an archive holding several games offers a choice");
    assert_eq!(source.scan(&app::data_roots::DataRoots::single(&data)).len(), 2);

    let lone = dir.join("solo.zip");
    write_zip(&lone, &[("amber.z5", amber())]);
    assert!(
        app::picker::StorySource::of(&lone, &app::data_roots::DataRoots::single(&data)).is_none(),
        "a one-game archive is not a menu",
    );

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// `--story` by entry name reaches the SECOND game — the whole point, and the
/// thing no harness, capture or bug report could do before.
///
/// Both halves are asserted: the pair the flag resolves to, and the bytes that
/// pair actually opens. Either alone is satisfiable by an accident — a selector
/// that is carried but never honoured looks identical to one that is.
#[test]
fn story_by_entry_name_opens_the_second_game_in_the_zip() {
    let dir = scratch("sq1098-pick");
    let data = scratch("sq1098-pick-data");
    let zip = two_game_zip(&dir);
    let source = app::picker::StorySource::of(&zip, &app::data_roots::DataRoots::single(&data)).expect("a source");

    let (path, entry) = app::story_pick::pick(Some(&source), &zip, &app::data_roots::DataRoots::single(&data), "beacon")
        .expect("the second game is reachable by name");
    assert_eq!(path, zip);
    assert_eq!(entry.as_deref(), Some("beacon.z5"));

    // The selector is honoured, not merely carried: r77 is Beacon's header and
    // r41 is what the archive's tiebreak would have opened.
    let (loaded, medium) = app::hints::load_mounted_story_from(&zip, entry.as_deref())
        .expect("the named entry loads");
    let bytes = loaded.bytes();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 77, "the game the name asked for");
    assert_eq!(medium, None, "a zip is not a pressed medium and must not claim one");

    // …and a number picks from the same list.
    let (_, first) = app::story_pick::pick(Some(&source), &zip, &app::data_roots::DataRoots::single(&data), "1").expect("in range");
    assert!(first.is_some(), "a numbered pick names its entry too");

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A miss must never fall back to booting an arbitrary game, and the refusal
/// must not call a download a disk.
#[test]
fn a_name_that_matches_nothing_in_the_zip_refuses_and_calls_it_an_archive() {
    let dir = scratch("sq1098-miss");
    let data = scratch("sq1098-miss-data");
    let zip = two_game_zip(&dir);
    let source = app::picker::StorySource::of(&zip, &app::data_roots::DataRoots::single(&data)).expect("a source");

    let err = app::story_pick::pick(Some(&source), &zip, &app::data_roots::DataRoots::single(&data), "trinity")
        .expect_err("a miss refuses rather than opening the first entry");
    assert!(err.starts_with("no story on this archive is named 'trinity':"), "{err}");
    assert!(err.contains("beacon"), "the menu rides along with the refusal: {err}");

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// An entry named at launch that is no longer in the archive must SAY SO rather
/// than open whichever story is (SQ-0859's rule, applied to the zip door).
///
/// This is the shape a stale cursor or a re-downloaded archive takes, and
/// silently opening a different game is the failure mode that looks correct.
#[test]
fn a_named_entry_that_is_gone_refuses_rather_than_opening_another_game() {
    let dir = scratch("sq1098-gone");
    let zip = dir.join("pack.zip");
    write_zip(&zip, &[("amber.z5", amber())]);

    let err = app::hints::load_mounted_story_from(&zip, Some("beacon.z5"))
        .expect_err("a name that is not in the archive is not a story");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(err.to_string().contains("beacon.z5"), "the error names what was asked for: {err}");

    // An entry that IS there but is not a game is refused the same way, rather
    // than falling through to the story sitting beside it.
    let mixed = dir.join("mixed.zip");
    write_zip(&mixed, &[("readme.txt", b"not a game".to_vec()), ("amber.z5", amber())]);
    let err = app::hints::load_mounted_story_from(&mixed, Some("readme.txt"))
        .expect_err("a named non-story does not resolve to its neighbour");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    // …while the archive still opens perfectly well without a selector.
    let (loaded, _) = app::hints::load_mounted_story_from(&mixed, None).expect("one game inside");
    assert_eq!(u16::from_be_bytes([loaded.bytes()[2], loaded.bytes()[3]]), 41);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A zip carrying one game and its RESOURCES is one row, not two — the shape
/// SQ-1085 built the zip tier for (`journey.z6` beside `Journey.blb`).
///
/// A resource-only Blorb is not a story and never becomes an entry, which is
/// what makes "classified by content" the right rule for enumeration as well as
/// for opening: a name-based scan would have listed the artwork as a game.
#[test]
fn a_game_packed_with_its_blorb_is_still_one_row() {
    let dir = scratch("sq1098-blorb");
    let data = scratch("sq1098-blorb-data");
    let zip = dir.join("amber-with-art.zip");
    write_zip(&zip, &[("amber.z5", amber()), ("Amber.blb", resource_only_blorb())]);

    let rows = app::picker::resolve_entries(&zip, &app::data_roots::DataRoots::single(&data));
    assert_eq!(rows.len(), 1, "the resources are not a second game: {rows:?}");
    assert_eq!(rows[0].meta.disk_entry, None, "one game means no selector");
    assert_eq!(rows[0].story_key(), "amber-with-art.zip");
    assert!(
        app::picker::StorySource::of(&zip, &app::data_roots::DataRoots::single(&data)).is_none(),
        "a game and its artwork is not a choice to make",
    );

    for d in [dir, data] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A Blorb carrying pictures and no executable — the shape a release `.blb` has.
fn resource_only_blorb() -> Vec<u8> {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    let pict = chunk(b"PNG ", &[0x89, b'P', b'N', b'G', 13, 10, 26, 10]);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Pict");
    ridx.extend_from_slice(&0u32.to_be_bytes());
    // Offset of the first chunk after `FORM<len>IFRS` + the RIdx chunk header
    // and payload.
    ridx.extend_from_slice(&((12 + 8 + ridx.len()) as u32).to_be_bytes());
    let mut body = Vec::new();
    body.extend_from_slice(b"IFRS");
    body.extend_from_slice(&chunk(b"RIdx", &ridx));
    body.extend_from_slice(&pict);
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}
