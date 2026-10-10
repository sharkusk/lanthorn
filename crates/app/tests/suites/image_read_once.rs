//! SQ-1762: opening one game off a disk image reads the image FILE once.
//!
//! A launch used to read the whole image four or five times — the story step,
//! the artwork step, the native-font step (twice) and the machine-detection
//! fallback each did their own `std::fs::read`. Over a network share the 354 MB
//! *Masterpieces* CD made that a minute apiece.
//!
//! Two guards, because each alone has a hole:
//!
//! * **The count.** Drive every step a launch takes against a real (synthetic)
//!   disk image and assert `blorb::image` read the file from disk exactly once.
//!   It cannot see a step that bypasses `blorb::image` and calls `fs::read`
//!   itself — which is what the second guard is for.
//! * **The scan.** No production file in `app` or `cli-host` may read a file with
//!   `fs::read` and then hand it to a disk-image mount or sniff. That is the
//!   shape every regression of this defect has had.
//!
//! FALSIFICATION: set `MAX_ENTRIES` to 0 in `crates/blorb/src/image.rs` (every
//! open now reads) and the count reaches 7 on this launch; revert a site to
//! `std::fs::read(path)` + `DiskImage::detect(&raw)` and the scan names it.

use std::path::{Path, PathBuf};

const SECTOR: usize = 2048;

/// A cooked ISO 9660 disc carrying `files` at its root: just enough volume for
/// the reader to mount it and for `DiskImage::detect` to name it.
fn iso(files: &[(&str, &[u8])]) -> Vec<u8> {
    const ROOT: usize = 18;
    let record = |id: &[u8], extent: u32, len: u32, flags: u8| -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[6..10].copy_from_slice(&extent.to_be_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[14..18].copy_from_slice(&len.to_be_bytes());
        r[25] = flags;
        r[32] = id.len() as u8;
        r.extend_from_slice(id);
        if id.len().is_multiple_of(2) {
            r.push(0);
        }
        r[0] = r.len() as u8;
        r
    };
    let mut records = vec![
        record(&[0], ROOT as u32, SECTOR as u32, 2),
        record(&[1], ROOT as u32, SECTOR as u32, 2),
    ];
    let mut at = ROOT + 1;
    let mut data: Vec<(usize, &[u8])> = Vec::new();
    for (name, bytes) in files {
        records.push(record(format!("{name};1").as_bytes(), at as u32, bytes.len() as u32, 0));
        data.push((at, bytes));
        at += bytes.len().div_ceil(SECTOR).max(1);
    }
    let mut image = vec![0u8; at * SECTOR];
    let pvd = 16 * SECTOR;
    image[pvd] = 1;
    image[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
    image[pvd + 6] = 1;
    image[pvd + 40..pvd + 72].fill(b' ');
    image[pvd + 40..pvd + 49].copy_from_slice(b"TEST DISC");
    image[pvd + 128..pvd + 130].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    image[pvd + 130..pvd + 132].copy_from_slice(&(SECTOR as u16).to_be_bytes());
    let root = record(&[0], ROOT as u32, SECTOR as u32, 2);
    image[pvd + 156..pvd + 156 + 34].copy_from_slice(&root[..34]);
    image[17 * SECTOR] = 255;
    image[17 * SECTOR + 1..17 * SECTOR + 6].copy_from_slice(b"CD001");
    let mut o = ROOT * SECTOR;
    for r in &records {
        image[o..o + r.len()].copy_from_slice(r);
        o += r.len();
    }
    for (block, bytes) in data {
        let at = block * SECTOR;
        image[at..at + bytes.len()].copy_from_slice(bytes);
    }
    image
}

fn story() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/zork1-mit.z3");
    std::fs::read(path).expect("unit_tests/zork1-mit.z3 is committed")
}

#[test]
fn the_synthetic_disc_is_one_the_launch_recognises() {
    let dir = app::scratch_dir("image-read-once-sanity");
    let path = dir.join("game.iso");
    std::fs::write(&path, iso(&[("STORY.DAT", &story())])).unwrap();
    blorb::image::clear_cache();
    let file = blorb::image::open_disk(&path).expect("a disk image");
    assert_eq!(file.format(), blorb::medium::DiskImage::Iso9660);
    let disk = blorb::medium::MountedDisk::mount_file(&file, Vec::new).expect("mounts");
    assert_eq!(disk.story().expect("a story").bytes, story());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_launch_off_a_disk_image_reads_the_file_once() {
    let dir = app::scratch_dir("image-read-once");
    let path = dir.join("game.iso");
    std::fs::write(&path, iso(&[("STORY.DAT", &story())])).unwrap();
    blorb::image::clear_cache();
    let reads = blorb::image::whole_reads_on_this_thread;
    let start = reads();

    // The picker's row for it, and the key its saves live under.
    let key = cli_host::storage::story_key_at(&path);
    assert!(!key.is_empty());
    match app::hints::scan_mounted_stories(&path) {
        app::hints::DiskScan::Stories(found) => assert_eq!(found.stories.len(), 1),
        _ => panic!("the picker sees one story on the disc"),
    }

    // The story itself.
    let mounted = app::hints::load_mounted_story_full(&path, None).expect("the story loads");
    assert_eq!(mounted.disk_image, Some(blorb::medium::DiskImage::Iso9660));

    // The artwork and sound step, and the font step.
    let files = app::assets::files(&path);
    assert!(
        files.iter().any(|f| f.is_on_medium() && f.name == "STORY.DAT"),
        "the disc's own files are listed"
    );
    assert_eq!(app::assets::volumes(&path).len(), 1);
    let request = app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile: app::interpreter::InterpreterProfile::IbmPc,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: None,
        disks: None,
    };
    let _ = app::native_font::resolve(&request);
    let _ = app::native_font::detected(&request);

    // Machine detection WITHOUT the mount's answer handed to it: the fallback.
    let (profile, _) =
        app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
    let _ = profile;

    assert_eq!(
        reads() - start,
        1,
        "every step of a launch must share the one read of the image file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Production Rust files under `dir`, skipping test trees.
fn production_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "tests" && !n.to_string_lossy().starts_with("target")) {
                production_sources(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The production part of `src`: everything before its `#[cfg(test)]` /
/// `#[cfg(all(test` module, which is where these files keep their tests.
fn production_part(src: &str) -> &str {
    let cut = ["\n#[cfg(test)]", "\n#[cfg(all(test"]
        .iter()
        .filter_map(|m| src.find(m))
        .min()
        .unwrap_or(src.len());
    &src[..cut]
}

#[test]
fn no_production_code_reads_an_image_file_itself() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    production_sources(&root.join("app/src"), &mut files);
    production_sources(&root.join("cli-host/src"), &mut files);
    assert!(files.len() > 50, "the scan found the sources: {}", files.len());

    // A mount or sniff of a disk image, and the one door a file may reach it by.
    const SINKS: [&str; 5] =
        ["DiskImage::detect(", "MountedDisk::mount(", "MountedDisk::mount_set(", "Hfs::mount(", "mount_at("];
    let mut offenders = Vec::new();
    for file in &files {
        let src = std::fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = production_part(&src).lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || !line.contains("fs::read(") {
                continue;
            }
            // The read and the sink within a few lines of each other: the shape
            // `let raw = fs::read(p)?; … DiskImage::detect(&raw) … mount(raw)`.
            let window = lines[i..lines.len().min(i + 6)].join("\n");
            if let Some(sink) = SINKS.iter().find(|s| window.contains(**s)) {
                offenders.push(format!("{}:{}: fs::read(..) feeds {sink}", file.display(), i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "read an image through blorb::image so the launch reads it once (SQ-1762):\n{}",
        offenders.join("\n")
    );
}
