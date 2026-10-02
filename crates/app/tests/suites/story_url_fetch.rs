//! SQ-1086 — a URL is accepted wherever a story path is.
//!
//! The unit tests in `app::story_url` cover recognition, filename derivation and
//! sanitisation. What only an integration suite can settle is the claim the
//! feature rests on: **the fetch hands its file to the ordinary loader, so every
//! filetype lanthorn already opens works without a second loader.** So these
//! cases fetch through a canned [`UrlSource`] — nothing here ever touches the
//! network — and then ask the real `hints::load_mounted_story` and the real
//! `picker::scan_stories` whether the result is a story and whether the library
//! can see it.
//!
//! Two of them read `stories/`, which is gitignored, and skip vacuously when it
//! is absent (CI has no such directory). The rest build their payloads in memory
//! and always run.

use std::path::{Path, PathBuf};

use app::story_url::{
    fetch_to_dir, keep_in_library, ArchiveImage, Fetched, FetchedArchive, FetchError, KeepMode,
    Payload, UrlSource,
};

// ── Harness ──────────────────────────────────────────────────────────────────

/// A [`UrlSource`] that hands back bytes already in hand. The whole network seam.
struct Canned {
    bytes: Vec<u8>,
    disposition: Option<String>,
}
impl UrlSource for Canned {
    fn get(&self, _url: &str) -> Result<Payload, FetchError> {
        Ok(Payload { disposition: self.disposition.clone(), bytes: self.bytes.clone() })
    }
}

fn canned(bytes: Vec<u8>) -> Canned {
    Canned { bytes, disposition: None }
}

/// Unwrap a fetch that must be a runnable story (SQ-1096). Every case that goes
/// on to say `.path` is also asserting THIS — that the download was something
/// the loader opens, not an archive lanthorn can only unpack.
fn story(got: Fetched) -> app::story_url::FetchedStory {
    match got {
        Fetched::Story(s) => s,
        Fetched::DiskImages(a) => {
            panic!("expected a story; got an archive of {} disk images", a.images.len())
        }
    }
}

/// …and the mirror image: a fetch that must be an archive of floppies.
fn archive(got: Fetched) -> FetchedArchive {
    match got {
        Fetched::DiskImages(a) => a,
        Fetched::Story(s) => panic!("expected an archive of disk images; got {}", s.filename()),
    }
}

/// A fresh scratch directory, named for the case so parallel cases cannot meet.
fn scratch(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("sq1086-{tag}"))
}

/// The gitignored library of commercial fixtures, or `None` — every case that
/// reads it skips vacuously so CI stays green (see `CLAUDE.md`, Test fixtures).
fn stories_dir() -> Option<PathBuf> {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    d.is_dir().then_some(d)
}

/// A minimal-but-coherent v5 story image. Every clause of
/// `blorb::adf::looks_like_zcode` is satisfied, because that is the predicate
/// both the fetch's content sniff and the real loader apply (SQ-0889).
fn zcode_v5() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 5;
    b[0x04..0x06].copy_from_slice(&96u16.to_be_bytes()); // high memory
    b[0x08..0x0A].copy_from_slice(&100u16.to_be_bytes()); // dictionary (static)
    b[0x0A..0x0C].copy_from_slice(&64u16.to_be_bytes()); // objects (dynamic)
    b[0x0C..0x0E].copy_from_slice(&70u16.to_be_bytes()); // globals (dynamic)
    b[0x0E..0x10].copy_from_slice(&96u16.to_be_bytes()); // static memory base
    b[0x12..0x18].copy_from_slice(b"890714"); // serial
    b[0x1A..0x1C].copy_from_slice(&(128u16 / 4).to_be_bytes()); // file length / 4
    b
}

/// A real ZIP holding one `.z5` entry — the shape `hints::read_story_file`
/// unwraps. Built rather than committed so the case never depends on a fixture.
fn zip_of_a_story() -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file(
            "curses.z5",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .expect("zip entry");
        zip.write_all(&zcode_v5()).expect("zip write");
        zip.finish().expect("zip finish");
    }
    buf.into_inner()
}

// ── The loader hand-off ──────────────────────────────────────────────────────

/// The whole claim of the feature, on the formats that need no fixture: fetch,
/// then hand the file to the loader lanthorn uses for a path typed by hand.
#[test]
fn a_fetched_z_code_image_opens_through_the_ordinary_loader() {
    let dir = scratch("zcode");
    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &dir)
        .expect("a coherent v5 image is fetchable");
    let got = story(got);
    assert_eq!(got.path, dir.join("curses.z5"));
    assert!(
        matches!(app::hints::load_mounted_story(&got.path), Ok((app::hints::LoadedStory::ZCode(_), None))),
        "the fetched file opens as Z-code, off the same path a typed one takes"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fetched_glulx_image_opens_through_the_ordinary_loader() {
    let dir = scratch("glulx");
    let mut glulx = b"Glul".to_vec();
    glulx.extend_from_slice(&[0u8; 60]);
    let got = fetch_to_dir(&canned(glulx), "https://example.org/if/kerkerkruip.ulx", &dir)
        .expect("a Glulx image is fetchable");
    let got = story(got);
    assert!(matches!(
        app::hints::load_mounted_story(&got.path),
        Ok((app::hints::LoadedStory::Glulx(_), None))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A ZIP is not unpacked by the fetch — it is written as a `.zip` and the LOADER
/// unwraps it, exactly as it does for a zip already sitting in a library. That is
/// what "hand the file to the existing loader" buys: no second unpacker.
#[test]
fn a_fetched_zip_is_unwrapped_by_the_loader_not_by_the_fetch() {
    let dir = scratch("zip");
    let got = fetch_to_dir(&canned(zip_of_a_story()), "https://example.org/if/curses.zip", &dir)
        .expect("a zip holding a story is fetchable");
    let got = story(got);
    assert_eq!(got.path.extension().unwrap(), "zip", "the archive keeps its own name");
    assert!(matches!(
        app::hints::load_mounted_story(&got.path),
        Ok((app::hints::LoadedStory::ZCode(_), None))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A download URL that names no file at all still lands under a name the library
/// can see — the extension comes from the bytes, which is the only source that
/// cannot lie about what the file is.
#[test]
fn an_extensionless_download_url_still_lands_openable() {
    let dir = scratch("noext");
    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/download.php?id=7", &dir)
        .expect("fetchable");
    let got = story(got);
    assert_eq!(got.path, dir.join("download.php.z5"));
    assert!(app::hints::load_mounted_story(&got.path).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// "A URL that is not a story at all" must be legible, not a crash and not an
/// empty picker: the message names what arrived, and nothing is written.
#[test]
fn a_login_page_is_refused_with_a_message_naming_what_arrived() {
    let dir = scratch("login");
    let page = b"<!DOCTYPE html><html><head><title>Sign in</title></head></html>".to_vec();
    let err = fetch_to_dir(&canned(page), "https://example.org/if/curses.z5", &dir)
        .expect_err("an HTML page is not a story");
    let msg = err.to_string();
    assert!(msg.contains("a web page"), "says what it fetched: {msg}");
    assert!(msg.contains("bytes"), "and how much of it: {msg}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "and writes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Keeping it ───────────────────────────────────────────────────────────────

/// The point of the keep prompt: after a keep, the directory the PICKER scans
/// holds the story. Asserted through `picker::scan_stories` itself rather than a
/// file-exists check, because "the next launch finds it" is a claim about the
/// scan, not about the filesystem.
#[test]
fn keeping_a_fetched_story_puts_it_where_the_picker_will_find_it() {
    let temp = scratch("keep-temp");
    let library = scratch("keep-lib");
    let data_base = scratch("keep-data");

    assert!(
        app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base)).is_empty(),
        "the library starts empty, so the row below can only come from the keep"
    );

    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &temp)
        .expect("fetchable");
    let got = story(got);
    let kept = keep_in_library(&got.path, &library, KeepMode::KeepBoth).expect("kept");
    assert_eq!(kept, library.join("curses.z5"));
    assert!(got.path.exists(), "the running game's own file survives the copy");

    let rows = app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base));
    assert_eq!(rows.len(), 1, "the picker lists exactly the story that was kept");
    assert_eq!(rows[0].path, kept);

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Declining keeps the library untouched — a fetch that is not kept must leave
/// no trace in the directory the picker reads.
#[test]
fn declining_leaves_the_library_exactly_as_it_was() {
    let temp = scratch("decline-temp");
    let library = scratch("decline-lib");
    let data_base = scratch("decline-data");

    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &temp)
        .expect("fetchable");
    let got = story(got);
    // No `keep_in_library` call is what "declined" means.
    assert!(app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base)).is_empty());
    assert!(got.path.exists(), "and it still plays from where it landed");

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

// ── Real media (skips vacuously without `stories/`) ──────────────────────────

/// The claim "every supported filetype works for free" against files that are
/// actually of those types — release disk images, Blorbs and bare story images
/// alike. Each one is replayed through the fetch as if it had come off the
/// network, and then opened.
///
/// A sample rather than the whole directory: the point is coverage of FORMATS,
/// and a few hundred megabytes of copies would be a slow test, not a better one.
#[test]
fn real_media_of_every_format_present_survives_a_round_trip_through_the_fetch() {
    let Some(stories) = stories_dir() else {
        eprintln!("skipping: no stories/ directory");
        return;
    };
    let dir = scratch("realmedia");

    // One specimen per extension, so the sample is a census of formats rather
    // than of filenames.
    let mut by_ext: std::collections::BTreeMap<String, (u64, PathBuf)> =
        std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&stories).into_iter().flatten().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        let ext = ext.to_ascii_lowercase();
        // Only formats the loader claims; anything else in the directory (a
        // README, a cover, a `.mg1`) is not a story and is not this test's
        // business.
        if !matches!(
            ext.as_str(),
            "z3" | "z4" | "z5" | "z6" | "z7" | "z8" | "ulx" | "blb" | "blorb" | "zblorb" | "gblorb"
                | "adf" | "d64" | "st" | "2mg" | "po" | "dsk" | "img" | "ima" | "image"
        ) {
            continue;
        }
        // Skip only what the fetch itself would refuse — the cap, not a number
        // of this test's own. That is deliberate: `Kerkerkruip.gblorb` is
        // 22,109,534 bytes and used to sit above BOTH, so a private limit here
        // would have quietly agreed with the defect (SQ-1086).
        if entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > app::ifdb_search::MAX_DOWNLOAD {
            continue;
        }
        // The LARGEST specimen of each format, not the first the directory
        // happens to hand back: the size cap is what this case most needs to
        // exercise, and `Kerkerkruip.gblorb` (22,109,534 bytes) is the file that
        // proves it — an arbitrary `.gblorb` of 200 KB proves nothing about it.
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match by_ext.get(&ext) {
            Some((seen, _)) if *seen >= len => {}
            _ => {
                by_ext.insert(ext, (len, path));
            }
        }
    }
    if by_ext.is_empty() {
        eprintln!("skipping: stories/ holds no readable specimen");
        return;
    }

    // The baseline has to be the file ISOLATED, not the file in `stories/`.
    // `hints::load_mounted_story` mounts a disk image with the other volumes of
    // its release beside it (SQ-0864), so `zork_zero_4.dsk` opens in a library
    // and holds no story at all on its own — and a fetch of one volume delivers
    // exactly that, one volume. Refusing it (legibly, having removed the file it
    // wrote) is the right answer, not a defect, so such a specimen is not this
    // case's business; probe each candidate alone and sweep only the ones that
    // are self-contained.
    let probe = scratch("realmedia-probe");
    let mut opened = 0usize;
    for (ext, (len, path)) in &by_ext {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("story");
        let alone = probe.join(name);
        if std::fs::write(&alone, &bytes).is_err() {
            continue;
        }
        let self_contained = app::hints::load_mounted_story(&alone).is_ok();
        let _ = std::fs::remove_file(&alone);
        if !self_contained {
            continue;
        }
        let url = format!("https://example.org/if/{name}");
        let got = fetch_to_dir(&canned(bytes), &url, &dir)
            .unwrap_or_else(|e| panic!("{ext}: {} did not survive a fetch: {e}", path.display()));
        let got = story(got);
        assert!(
            app::hints::load_mounted_story(&got.path).is_ok(),
            "{ext}: fetched {} does not open, though the original does",
            path.display()
        );
        opened += 1;
        // Name the specimen, as any fixture-driven case here does — the sizes
        // are the whole point of the cap this exercises.
        eprintln!("  {ext}: {} ({len} bytes)", path.display());
    }
    assert!(opened > 0, "non-vacuity: at least one real specimen must have been exercised");
    eprintln!("round-tripped {opened} specimens of {} candidate formats", by_ext.len());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&probe);
}

// ── The cap is a measurement, not an impression (SQ-1086) ────────────────────

/// `MAX_DOWNLOAD` used to be 16 MiB under a comment asserting that "a real
/// Z-code/Glulx/blorb rarely exceeds a few MiB", which refused *Kerkerkruip*.
/// This case is what stops the number drifting back into an opinion: it reads
/// the corpus off the disk and checks the constant against it.
///
/// The cap-versus-constant half needs no test at all: `ifdb_search` carries a
/// `const _: () = assert!(MAX_DOWNLOAD > corpus::LARGEST_GAME)`, so lowering
/// the cap is a BUILD failure rather than a test failure. What only a test can
/// check is the constant against the corpus it claims to describe — and that
/// needs the gitignored `stories/`, so it skips vacuously.
#[test]
fn the_download_cap_admits_every_game_in_the_corpus() {
    let Some(stories) = stories_dir() else {
        eprintln!("skipping the corpus half: no stories/ directory");
        return;
    };
    let mut largest: (u64, String) = (0, String::new());
    for entry in std::fs::read_dir(&stories).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        if !matches!(
            ext.to_ascii_lowercase().as_str(),
            "z3" | "z4" | "z5" | "z6" | "z7" | "z8" | "ulx" | "blb" | "blorb" | "zblorb" | "gblorb"
        ) {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if len > largest.0 {
            largest = (len, path.display().to_string());
        }
    }
    if largest.0 == 0 {
        eprintln!("skipping the corpus half: no bare story files present");
        return;
    }
    assert!(
        app::ifdb_search::MAX_DOWNLOAD > largest.0,
        "the cap ({}) refuses {} ({} bytes) — raise it, and raise corpus::LARGEST_GAME with it",
        app::ifdb_search::MAX_DOWNLOAD,
        largest.1,
        largest.0,
    );
    assert!(
        app::corpus::LARGEST_GAME >= largest.0,
        "corpus::LARGEST_GAME ({}) is stale: {} is {} bytes",
        app::corpus::LARGEST_GAME,
        largest.1,
        largest.0,
    );
}

// ── A kept archive is visible (SQ-1086) ─────────────────────────────────────

/// The defect this pair exists for: the keep prompt says "it will be there next
/// time", and before `picker::has_story_ext` admitted `.zip` that was false for
/// an archive — the file landed in the library and the only view that would show
/// it did not list it.
#[test]
fn a_kept_zip_is_visible_to_the_picker() {
    let temp = scratch("zipkeep-temp");
    let library = scratch("zipkeep-lib");
    let data_base = scratch("zipkeep-data");

    let got = fetch_to_dir(&canned(zip_of_a_story()), "https://example.org/if/curses.zip", &temp)
        .expect("a zip holding a story is fetchable");
    let got = story(got);
    let kept = keep_in_library(&got.path, &library, KeepMode::KeepBoth).expect("kept");
    assert_eq!(kept.extension().unwrap(), "zip");

    let rows = app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base));
    assert_eq!(rows.len(), 1, "the archive the player kept is listed");
    assert_eq!(rows[0].path, kept);

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// …and admitting `.zip` must not turn every archive into a row. The scan opens
/// each candidate and drops it unless a story comes out, so an archive of
/// something else appears nowhere — the property that makes a generic extension
/// safe to admit at all.
#[test]
fn an_archive_holding_no_story_is_not_listed() {
    use std::io::Write;
    let library = scratch("zipjunk-lib");
    let data_base = scratch("zipjunk-data");

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file("holiday.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"not a story, just a note").unwrap();
        zip.finish().unwrap();
    }
    std::fs::write(library.join("photos.zip"), buf.into_inner()).unwrap();

    assert!(
        app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base)).is_empty(),
        "an archive with no story in it costs one open and yields no row"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Admitting `.zip` to the scan must not turn a hint ARCHIVE into a game row.
///
/// `hints.rs` explicitly expects a hint zip beside a story (its sidecar
/// resolution scans sibling zips), and the loose `deadlineinv.z5` form has
/// always folded into the game's row rather than listing as a game of its own.
/// The archive is the same file in a wrapper and now folds the same way — the
/// alternative, observed before the fix, was `deadline-hints.zip` sitting in the
/// list as a playable title next to the very story it belongs to.
#[test]
fn a_hint_archive_folds_into_its_game_rather_than_listing_as_one() {
    let library = scratch("hintzip-lib");
    let data_base = scratch("hintzip-data");

    std::fs::write(library.join("deadline.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadline-hints.zip"), zip_of_a_story()).unwrap();

    let rows = app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base));
    assert_eq!(rows.len(), 1, "one game, not a game plus its clues: {rows:?}");
    assert_eq!(rows[0].filename, "deadline.z5");
    assert_eq!(
        rows[0].hint_sidecar.as_deref(),
        Some(library.join("deadline-hints.zip").as_path()),
        "and the archive is attached as that game's hints"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// …and a LOOSE hint file still wins when both are present, so admitting `.zip`
/// cannot change which sidecar an existing library already resolves. An archive
/// is only ever chosen where nothing loose answered — the case that had no
/// answer at all before.
#[test]
fn a_loose_hint_file_still_outranks_an_archived_one() {
    let library = scratch("hintboth-lib");
    let data_base = scratch("hintboth-data");

    std::fs::write(library.join("deadline.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadlineinv.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadline-hints.zip"), zip_of_a_story()).unwrap();

    let rows = app::picker::scan_stories(&library, &app::data_roots::DataRoots::single(&data_base));
    let game = rows.iter().find(|r| r.filename == "deadline.z5").expect("the game is listed");
    assert_eq!(
        game.hint_sidecar.as_deref(),
        Some(library.join("deadlineinv.z5").as_path()),
        "the loose sidecar is chosen, exactly as it was before .zip joined the scan"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

// ── SQ-1096: a downloaded zip of release disk images ─────────────────────────
//
// The measured symptom: `TRINITY1.D64` and `Arthur - The Quest for
// Excalibur.adf` both boot loose and were both refused inside a zip, with
//
//     no story file inside the zip …/trin.zip (1 entry read; none is a Blorb,
//     a Z-machine story, a Glulx image or a Scott Adams database)
//
// which is accurate and useless: the zip classifier knows four kinds of story
// and no media at all. The loader is unchanged — a zip is still a volume of raw
// stories classified by CONTENT. What changed is the FETCH, which now recognises
// an archive of floppies by extension and offers to unpack it.
//
// The synthetic cases below carry junk inside disk-image NAMES on purpose: the
// whole claim is that the fetch classifies by extension here, so a case that
// leaned on real sectors would be testing the mount instead. The two commercial
// specimens get their own case and skip vacuously.

/// A real ZIP with the given (stored name, bytes) entries, stored uncompressed.
fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, bytes) in entries {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .expect("zip entry");
            zip.write_all(bytes).expect("zip write");
        }
        zip.finish().expect("zip finish");
    }
    buf.into_inner()
}

/// Bytes that are emphatically not a story of any of the four kinds — so that
/// the only thing that can classify an entry carrying them is its NAME.
fn not_a_story(tag: u8) -> Vec<u8> {
    let mut v = vec![0xE5u8; 4096];
    v[0] = tag;
    v
}

/// The measured symptom, and the fix for it: a zip whose only entry is a disk
/// image is no longer "no story file inside the zip" — it is an archive the
/// fetch recognises and hands on to be unpacked.
#[test]
fn a_zip_of_one_disk_image_is_recognised_rather_than_refused() {
    let dir = scratch("sq1096-one");
    let payload = zip_of(&[("TRINITY1.D64", not_a_story(1))]);
    let got = archive(
        fetch_to_dir(&canned(payload), "https://example.org/if/trin.zip", &dir)
            .expect("an archive of floppies is fetchable"),
    );
    assert_eq!(got.names(), vec!["TRINITY1.D64".to_string()], "the image is found by extension");
    assert!(got.path.exists(), "the archive is kept until the prompt is answered");
    assert_eq!(got.path.extension().unwrap(), "zip");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A multi-disk release is the NORMAL shape of a zipped disk image, not the
/// exception — and all of it comes out, flattened, because `disk_set::mount_at`
/// finds an image's siblings in the directory it sits in. Images left in
/// `Journey/disks/` would be a five-floppy release that mounts as one floppy.
#[test]
fn a_multi_disk_release_unpacks_whole_and_flattened() {
    let dir = scratch("sq1096-many-dl");
    let library = scratch("sq1096-many-lib");
    let entries: Vec<(&str, Vec<u8>)> = vec![
        ("Journey/disks/journey_s3.dsk", not_a_story(3)),
        ("Journey/disks/journey_s1.dsk", not_a_story(1)),
        ("Journey/disks/journey_s2.dsk", not_a_story(2)),
        ("Journey/disks/journey_s4.dsk", not_a_story(4)),
        ("Journey/disks/journey_s5.dsk", not_a_story(5)),
    ];
    let got = archive(
        fetch_to_dir(&canned(zip_of(&entries)), "https://example.org/if/journey.zip", &dir)
            .expect("fetchable"),
    );
    assert_eq!(got.images.len(), 5, "every volume of the release is found");

    let written = app::story_url::unpack_disk_images(&got, &library, KeepMode::KeepBoth)
        .expect("unpacked");
    assert_eq!(written.len(), 5);
    let mut names: Vec<String> =
        written.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["journey_s1.dsk", "journey_s2.dsk", "journey_s3.dsk", "journey_s4.dsk", "journey_s5.dsk"],
        "flattened to basenames, so the five are siblings"
    );
    for p in &written {
        assert_eq!(p.parent(), Some(library.as_path()), "every image is a direct child");
    }
    assert!(!library.join("Journey").exists(), "no directory from the archive is recreated");
    // The one that would be launched is the first BY NAME, which for a release
    // named in reading order is disk 1 — and the rest are found beside it.
    assert_eq!(written[0].file_name().unwrap(), "journey_s1.dsk");
    // Contents, not just names: entry order is not name order above, so this
    // would catch a mapping that unpacked the right count under wrong names.
    assert_eq!(std::fs::read(&written[0]).unwrap()[0], 1);
    assert_eq!(std::fs::read(&written[4]).unwrap()[0], 5);

    for d in [dir, library] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// **The whitelist is the feature.** An arbitrary archive from an arbitrary URL
/// is untrusted input, the library directory is scanned on every launch, and the
/// offer on screen says "keep this game", not "unpack this archive". So exactly
/// the supported disk images come out and nothing else does — no readme, no
/// cover scan, no manual, no nested directory, and not even a file whose name
/// says `.z5`.
#[test]
fn only_supported_disk_images_are_extracted_and_nothing_else_is() {
    let dir = scratch("sq1096-junk-dl");
    let library = scratch("sq1096-junk-lib");
    let entries: Vec<(&str, Vec<u8>)> = vec![
        ("readme.txt", b"Scanned by somebody, 1994.\n".to_vec()),
        ("disk1.d64", not_a_story(1)),
        ("art/cover.png", not_a_story(0x89)),
        ("disk2.d64", not_a_story(2)),
        ("docs/manual.pdf", not_a_story(0x25)),
        ("bonus.z5", not_a_story(5)),
        ("INSTALL.EXE", not_a_story(0x4D)),
        ("notes.nfo", b"...".to_vec()),
    ];
    let got = archive(
        fetch_to_dir(&canned(zip_of(&entries)), "https://example.org/if/bundle.zip", &dir)
            .expect("fetchable"),
    );
    assert_eq!(
        got.names(),
        vec!["disk1.d64".to_string(), "disk2.d64".to_string()],
        "the count the player is told is the count that will be written"
    );

    let written = app::story_url::unpack_disk_images(&got, &library, KeepMode::KeepBoth)
        .expect("unpacked");
    assert_eq!(written.len(), 2);
    let mut got_names: Vec<String> = std::fs::read_dir(&library)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    got_names.sort();
    assert_eq!(
        got_names,
        vec!["disk1.d64", "disk2.d64"],
        "the library holds the two images and NOTHING else out of that archive"
    );

    for d in [dir, library] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// A zip holding both a story the loader can run and some disk images is the
/// STORY's — decided by asking the loader first, so it is never entry order that
/// settles it.
#[test]
fn a_story_inside_the_zip_wins_over_the_disk_images_beside_it() {
    let dir = scratch("sq1096-both");
    let entries: Vec<(&str, Vec<u8>)> = vec![
        ("disks/side_a.d64", not_a_story(1)),
        ("curses.z5", zcode_v5()),
        ("disks/side_b.d64", not_a_story(2)),
    ];
    let got = story(
        fetch_to_dir(&canned(zip_of(&entries)), "https://example.org/if/curses.zip", &dir)
            .expect("fetchable"),
    );
    assert!(
        matches!(
            app::hints::load_mounted_story(&got.path),
            Ok((app::hints::LoadedStory::ZCode(_), None))
        ),
        "the loader's own answer, not the archive's entry order"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing may be written outside the library. A name that climbs out is refused
/// OUTRIGHT — the whole archive, not the one entry — rather than trimmed to its
/// final component, because an archive that tries it has said what it is.
#[test]
fn an_entry_that_climbs_out_of_the_library_is_refused_outright() {
    let dir = scratch("sq1096-climb");
    let entries: Vec<(&str, Vec<u8>)> =
        vec![("good.d64", not_a_story(1)), ("../../../evil.d64", not_a_story(2))];
    let err = fetch_to_dir(&canned(zip_of(&entries)), "https://example.org/if/x.zip", &dir)
        .expect_err("a traversal name is refused");
    assert!(matches!(err, FetchError::UnsafeEntry(_)), "refused as unsafe, not as unopenable: {err}");
    assert!(err.to_string().contains("evil.d64"), "the offending entry is named: {err}");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "and the download itself is not left lying about"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Declining writes nothing. Recognition is a read of the archive and no more —
/// the library is untouched until `unpack_disk_images` is called, which is what
/// answering "yes" does.
#[test]
fn recognising_an_archive_writes_nothing_into_the_library() {
    let dir = scratch("sq1096-decline-dl");
    let library = scratch("sq1096-decline-lib");
    let payload = zip_of(&[("disk1.d64", not_a_story(1)), ("disk2.d64", not_a_story(2))]);
    let got = archive(
        fetch_to_dir(&canned(payload), "https://example.org/if/x.zip", &dir).expect("fetchable"),
    );
    assert_eq!(got.images.len(), 2);
    assert_eq!(std::fs::read_dir(&library).unwrap().count(), 0, "nothing kept without an answer");
    assert!(!app::story_url::archive_collision(&got, &library), "and nothing to collide with");

    for d in [dir, library] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The collision the prompt's third button exists for, on a set: a name the
/// library already holds is either replaced or kept beside, never silently
/// clobbered and never silently renamed.
#[test]
fn a_name_the_library_already_holds_is_replaced_or_kept_beside() {
    let dir = scratch("sq1096-coll-dl");
    let library = scratch("sq1096-coll-lib");
    std::fs::write(library.join("disk1.d64"), b"the file the player already had").unwrap();

    let payload = zip_of(&[("disk1.d64", not_a_story(1)), ("disk2.d64", not_a_story(2))]);
    let got = archive(
        fetch_to_dir(&canned(payload), "https://example.org/if/x.zip", &dir).expect("fetchable"),
    );
    assert!(app::story_url::archive_collision(&got, &library), "the prompt is told to say so");

    let kept = app::story_url::unpack_disk_images(&got, &library, KeepMode::KeepBoth)
        .expect("unpacked");
    assert!(kept.contains(&library.join("disk1-2.d64")), "kept beside: {kept:?}");
    assert_eq!(
        std::fs::read(library.join("disk1.d64")).unwrap(),
        b"the file the player already had",
        "keeping both leaves the player's own file alone"
    );

    let replaced = app::story_url::unpack_disk_images(&got, &library, KeepMode::Replace)
        .expect("unpacked");
    assert!(replaced.contains(&library.join("disk1.d64")));
    assert_eq!(std::fs::read(library.join("disk1.d64")).unwrap()[0], 1, "now the archive's copy");

    for d in [dir, library] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Two entries that flatten onto one name are NOT a replace, whatever the player
/// answered about their own library: nothing in "replace the file I already had"
/// means "overwrite the disk you just unpacked with the next one out of the same
/// zip".
#[test]
fn two_entries_that_flatten_onto_one_name_land_side_by_side() {
    let dir = scratch("sq1096-flat-dl");
    let library = scratch("sq1096-flat-lib");
    let payload =
        zip_of(&[("side_a/disk1.d64", not_a_story(1)), ("side_b/disk1.d64", not_a_story(2))]);
    let got = archive(
        fetch_to_dir(&canned(payload), "https://example.org/if/x.zip", &dir).expect("fetchable"),
    );
    let written = app::story_url::unpack_disk_images(&got, &library, KeepMode::Replace)
        .expect("unpacked");
    assert_eq!(written.len(), 2, "both survive: {written:?}");
    assert_eq!(std::fs::read(library.join("disk1.d64")).unwrap()[0], 1);
    assert_eq!(std::fs::read(library.join("disk1-2.d64")).unwrap()[0], 2);

    for d in [dir, library] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// The two specimens that reproduced the bug, zipped and put back through the
/// whole chain: URL → zip → unpack → library → mount → story. Skips vacuously
/// without `stories/` (gitignored; CI has none).
///
/// **Trinity is a RELEASE, not a platter, and this case is where that stops
/// being a slogan.** `TRINITY1.D64` on its own answers *"no story file on the
/// disk image (0 files on TRINITY; is this the boot disk?)"* — measured here —
/// and mounts only with `TRINITY2.D64` beside it, because `disk_set::mount_at`
/// scans the directory the named image sits in. So "unpack all of them" is not a
/// convenience: unpack one volume of this release and the game does not run.
/// It is also why the images are flattened, since a set in a subdirectory is not
/// a set of siblings.
#[test]
fn the_commercial_specimens_survive_the_whole_chain() {
    let Some(stories) = stories_dir() else {
        eprintln!("skip: no stories/ directory");
        return;
    };
    // (label, the whole release, the volume the launch should reach for)
    let specimens: [(&str, &[&str], &str); 2] = [
        ("Trinity (C64)", &["TRINITY1.D64", "TRINITY2.D64"], "TRINITY1.D64"),
        (
            "Arthur (Amiga)",
            &["Arthur - The Quest for Excalibur.adf"],
            "Arthur - The Quest for Excalibur.adf",
        ),
    ];
    let mut ran = 0usize;
    for (label, volumes, boot) in specimens {
        let mut entries: Vec<(&str, Vec<u8>)> = Vec::new();
        for v in volumes {
            let Ok(bytes) = std::fs::read(stories.join(v)) else { break };
            entries.push((v, bytes));
        }
        if entries.len() != volumes.len() {
            eprintln!("skip {label}: not every volume is present");
            continue;
        }
        let dir = scratch(&format!("sq1096-real{ran}-dl"));
        let library = scratch(&format!("sq1096-real{ran}-lib"));
        let got = archive(
            fetch_to_dir(&canned(zip_of(&entries)), "https://example.org/if/press.zip", &dir)
                .unwrap_or_else(|e| panic!("{label}: zipped and refused: {e}")),
        );
        assert_eq!(got.names().len(), volumes.len(), "{label}: every volume is recognised");
        let written = app::story_url::unpack_disk_images(&got, &library, KeepMode::KeepBoth)
            .expect("unpacked");
        assert_eq!(written.len(), volumes.len());
        assert_eq!(
            written[0],
            library.join(boot),
            "{label}: the volume the launch reaches for is the first BY NAME"
        );
        assert!(
            app::hints::load_mounted_story(&written[0]).is_ok(),
            "{label}: unpacked out of the zip, the release mounts as it does loose"
        );
        eprintln!("  {label}: {} volume(s) zipped, unpacked, mounted", volumes.len());
        ran += 1;
        for d in [dir, library] {
            let _ = std::fs::remove_dir_all(&d);
        }
    }
    if ran == 0 {
        eprintln!("skip: no specimen present");
    }
}

/// The half of the Trinity measurement the case above rests on, stated on its
/// own so a future reader does not have to take it on trust: one volume of a
/// two-disk release does not mount alone. Skips vacuously.
#[test]
fn one_volume_of_a_release_does_not_mount_without_its_sibling() {
    let Some(stories) = stories_dir() else {
        eprintln!("skip: no stories/ directory");
        return;
    };
    let (Ok(one), Ok(two)) = (
        std::fs::read(stories.join("TRINITY1.D64")),
        std::fs::read(stories.join("TRINITY2.D64")),
    ) else {
        eprintln!("skip: Trinity's two D64s are not both present");
        return;
    };
    let library = scratch("sq1096-siblings");
    let solo = library.join("TRINITY1.D64");
    std::fs::write(&solo, &one).unwrap();
    let err = app::hints::load_mounted_story(&solo)
        .expect_err("side 1 alone carries no story — this is the premise of the case above");
    assert!(err.to_string().contains("is this the boot disk?"), "{err}");
    std::fs::write(library.join("TRINITY2.D64"), &two).unwrap();
    assert!(
        app::hints::load_mounted_story(&solo).is_ok(),
        "…and mounts the moment its sibling is beside it"
    );
    let _ = std::fs::remove_dir_all(&library);
}

/// The names are `blorb::medium`'s, not a list written here — so a format the
/// mount learns to read is one the fetch unpacks, with no second table to update.
#[test]
fn the_recognised_spellings_are_the_mediums_own() {
    for ext in ["d64", "adf", "dsk", "po", "img", "2mg", "iso", "st"] {
        assert!(
            blorb::medium::image_extensions().any(|e| e == ext),
            "{ext} is expected to be a supported image"
        );
        assert!(app::story_url::is_disk_image_name(&format!("release.{ext}")));
        assert!(
            app::story_url::is_disk_image_name(&format!("RELEASE.{}", ext.to_uppercase())),
            "archives are written on every platform there is"
        );
    }
    for name in ["readme.txt", "cover.png", "manual.pdf", "curses.z5", "game.zip", "noext", ".d64"] {
        assert!(!app::story_url::is_disk_image_name(name), "{name} must not be extracted");
    }
}
