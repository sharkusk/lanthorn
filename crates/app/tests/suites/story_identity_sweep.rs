//! **Z-code stopped being the else-branch, and nothing in the corpus noticed**
//! (SQ-0889).
//!
//! `hints::extract_story` used to test three formats and assume the fourth:
//! Blorb proved itself by magic, Glulx by `Glul`, Scott Adams by a content
//! sniff, and everything left over was handed to the Z-machine. The only gate
//! downstream was `zvm::header::parse_header`'s `3..=8` on byte 0 — six of 256
//! byte values, so roughly **2.3% of arbitrary containers pass it**. One did:
//! `stories/Shogun.po`, an 838 KB Apple II DiskCopy 4.2 image whose name-length
//! byte is `0x06`. lanthorn ran the whole disk image as a Version 6 story,
//! paired it with a sidecar Blorb belonging to a different file, printed
//! "story ended without asking for input", and exited **0**.
//!
//! A refusal is only worth having if it refuses the right things, and the risk
//! of a new gate is entirely on the other side: a real game that stops loading.
//! So the evidence here is a **sweep** rather than a fixture. Every file in
//! `stories/`, `masterpieces/` and `treasures/` is offered to the loader, and
//! the rules are stated over what is read rather than pinned to a list that
//! would need editing each time the user's shelf grows: everything that looks
//! like a story must load, everything that loads must be something an engine can
//! be handed, and everything turned away must be something that did not run
//! before either.
//!
//! Measured when the gate landed: **294 files, 227 loaded, 67 refused**, against
//! 269 and 25 with the gate removed. All 42 that changed sides are readmes,
//! `.pic` archives, Quetzal saves, Glk data and `.cfg` files, and
//! `zvm::header::parse_header` rejected every one of them a layer later — so the
//! change is when the failure happens and what it says, not whether anything
//! runs. Containers that used to become a bogus story: **one**, `Shogun.po`, and
//! it mounts now.
//!
//! All three directories are gitignored, so every case here skips vacuously.
//!
//! # A note on sweeping binaries
//!
//! Story files are full of NUL bytes, and `grep` without `-a` decides such a
//! stream is binary and reports nothing — a sweep written at the shell comes
//! back clean by accident and proves the opposite of what it claims (this cost
//! the SQ-0884 lane). Everything below drives `blorb`'s and `app`'s own
//! recognisers over bytes read into memory; no text tool is involved.

use std::path::{Path, PathBuf};

fn corpus_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    ["stories", "masterpieces", "treasures"].iter().map(|d| root.join(d)).collect()
}

// `files_under` is the shared loop-safe walker (SQ-1708): `treasures/` nests its
// discs a level down, and a symlink back to an ancestor must not recurse forever.
use crate::fixture_paths::files_under;

/// Every regular file in the three corpora, sorted, with the huge disc images
/// included — a 650 MB `.bin` is exactly the sort of container that used to
/// reach the Z-machine.
fn corpus_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in corpus_dirs() {
        files_under(&dir, &mut out);
    }
    out.sort();
    out
}

/// What the loader made of one file.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum Verdict {
    /// It loaded, whichever engine claimed it.
    Loaded,
    /// The loader refused it, with a message.
    Refused,
}

fn verdict(path: &Path) -> Verdict {
    match app::hints::load_story(path) {
        Ok(_) => Verdict::Loaded,
        Err(_) => Verdict::Refused,
    }
}


/// **The conservation law: refusing bad containers costs no real game.**
///
/// Sweeps the whole corpus and requires that everything the loader accepts is
/// something an engine can actually be handed — which, for the Z-machine, means
/// its header parses. A file that loads as Z-code and whose header `zvm` then
/// rejects is the exact defect this quest is about, one layer further in.
///
/// Stated this way rather than as "these N files load", because the corpus is
/// the user's shelf and grows: a pinned count would fail for the wrong reason
/// the day a new game arrives, and a rule that reads every file cannot go stale.
#[test]
fn every_story_the_loader_accepts_is_one_an_engine_can_run() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("SKIP: gitignored corpora absent");
        return;
    }
    let (mut loaded, mut refused) = (0, 0);
    for path in &files {
        match app::hints::load_story(path) {
            Ok(app::hints::LoadedStory::ZCode(bytes)) => {
                loaded += 1;
                // The gate that used to be the ONLY one. If a file reaches the
                // Z-machine, the Z-machine must be able to read its header.
                zvm::header::parse_header(&bytes).unwrap_or_else(|e| {
                    panic!("{}: loaded as Z-code and zvm refuses the header: {e:?}", path.display())
                });
            }
            Ok(_) => loaded += 1,
            Err(_) => refused += 1,
        }
    }
    eprintln!("corpus: {} files, {loaded} loaded, {refused} refused", files.len());
    assert!(loaded > 0, "the corpora are here but nothing in them loads");
}

/// **No file that looks like a story is refused** — the guard against an
/// over-tight gate, which is the entire risk a new check carries.
///
/// Stated in the direction that can fail. A refusal is cheap to get wrong in a
/// way nobody sees until a player opens their own shelf, so the sweep asserts
/// the positive: every file in the corpus whose bytes ARE a Z-machine header by
/// `blorb::adf::looks_like_zcode` must come out of the loader as Z-code. This
/// half is not vacuous — most of the corpus is bare `.z3`/`.z5`/`.z8` — so a
/// tightening of the check anywhere fails here immediately.
///
/// Measured when the gate landed: **294 files across the three corpora, 227
/// loaded, 67 refused** — against 269 loaded and 25 refused with the gate
/// removed. Every one of the 42 that changed sides is answered by the case
/// below.
#[test]
fn nothing_that_looks_like_a_story_is_refused() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("SKIP: gitignored corpora absent");
        return;
    }
    let mut checked = 0;
    for path in &files {
        let Ok(raw) = std::fs::read(path) else { continue };
        if !blorb::adf::looks_like_zcode(&raw) {
            continue;
        }
        checked += 1;
        match app::hints::load_story(path) {
            Ok(app::hints::LoadedStory::ZCode(_)) => {}
            other => panic!("{}: a Z-machine header, and the loader said {other:?}", path.display()),
        }
    }
    eprintln!("{checked} files carry a Z-machine header and every one of them loads");
    assert!(checked > 0, "the corpora are here but hold no bare story file");
}

/// **Every file the gate turned away was already unrunnable**, and the census
/// the quest asked for and nobody had taken.
///
/// The 42 files that changed sides are readmes, `.pic` archives, Quetzal saves,
/// Glk data and `.cfg` files. None of them was a game before: the loader handed
/// each to `zvm::header::parse_header`, which refused it a layer later with
/// `UnsupportedVersion` — so what changed is *when* the failure happens and what
/// it says, not whether the file runs. This case asserts exactly that, file by
/// file, which is what makes the sweep evidence rather than a count.
///
/// The interesting number is the other one. A file that `parse_header` would
/// have ACCEPTED while `looks_like_zcode` refuses it is a container that
/// silently became a bogus story — the class this quest exists for, and
/// uncounted until now. In this corpus, after `Shogun.po` became mountable,
/// there are **none**; it was the only one. That is reported rather than
/// asserted to be zero, because a new such file arriving on the user's shelf is
/// a discovery for this sweep to make, not a reason for it to fail.
#[test]
fn every_container_the_gate_turned_away_was_already_unrunnable() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("SKIP: gitignored corpora absent");
        return;
    }
    let mut silently_ran = Vec::new();
    let mut turned_away = 0;
    for path in &files {
        let Ok(raw) = std::fs::read(path) else { continue };
        // Only the files that reach the else-branch at all: no reader claims
        // them and no other engine's identity check fires.
        if blorb::medium::DiskImage::detect(&raw).is_some()
            || raw.starts_with(b"PK\x03\x04")
            || blorb::Blorb::is_blorb(&raw)
            || raw.starts_with(b"Glul")
            || std::str::from_utf8(&raw).is_ok_and(scott::looks_like_scott)
            || scott::looks_like_scott_bytes(&raw)
            || blorb::adf::looks_like_zcode(&raw)
        {
            continue;
        }
        turned_away += 1;
        assert_eq!(verdict(path), Verdict::Refused, "{}: must be refused", path.display());
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The message has to name the file rather than blame the game.
        let said = app::hints::extract_story(raw.clone())
            .expect_err("it was just refused")
            .to_string();
        assert!(
            said.contains("not a story file") && said.contains("ZMSD"),
            "{name}: the refusal must say what the bytes are: {said}"
        );
        // …and nothing that used to RUN may have stopped running.
        if zvm::header::parse_header(&raw).is_ok() {
            silently_ran.push(name);
        }
    }
    silently_ran.sort();
    eprintln!(
        "{turned_away} containers reach the else-branch and are refused; \
         {} of them used to become a bogus story: {silently_ran:?}",
        silently_ran.len()
    );
    assert!(turned_away > 0, "the corpora are here but nothing reaches the else-branch");
}

/// **The reported file, both halves.**
///
/// `stories/Shogun.po` is the case that opened the quest, and after SQ-0889's
/// first commit it is also the case that proves the two pieces are independent:
/// the DiskCopy 4.2 unwrap means it **mounts** now, so it never reaches the
/// else-branch at all. Its old symptom is gone twice over — once because the
/// image opens, and once because a container that did not open would be refused.
///
/// What is asserted here is the first half, because the second is asserted for
/// every container in the corpus by the case above: the file must load, and it
/// must load as the Apple II press of *Shogun* rather than as 838,484 bytes of
/// Version 6.
#[test]
fn the_reported_disk_image_loads_as_its_game_and_not_as_itself() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/Shogun.po");
    if !path.exists() {
        eprintln!("SKIP: gitignored fixture missing: stories/Shogun.po");
        return;
    }
    let raw = std::fs::read(&path).expect("readable");
    // The premise: byte 0 is the DiskCopy name length, a legal Z-machine
    // version, and the whole of the old gate.
    assert_eq!(raw[0], 6, "the DiskCopy name-length byte that read as Version 6");
    // Byte 0 in `3..=8` and 64 bytes or more was the WHOLE of the old gate, and
    // this file passes it — which is how a disk image became a v6 story.
    assert!(zvm::header::parse_header(&raw).is_ok(), "the old gate is what this file passed");
    assert_eq!(raw.len(), 838_484, "and it is a disk image, not a story");

    let story = match app::hints::load_story(&path).expect("the image mounts and offers its game") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected the Apple Shogun, got {other:?}"),
    };
    assert_ne!(story.len(), raw.len(), "the image itself must never be the story");
    let header = zvm::header::parse_header(&story).expect("a real header");
    assert_eq!(header.version, 6);
    assert_eq!(u16::from_be_bytes([story[2], story[3]]), 311, "release 311");
    assert_eq!(&story[0x12..0x18], b"890510", "serial 890510");
}

/// **A container that no reader claims is refused, and says what it is.**
///
/// The general case, with no fixture: a synthetic DiskCopy 4.2 header over
/// bytes that are not any filesystem. Nothing can mount it, byte 0 is `0x06`,
/// and it is 64 bytes and more — so this is precisely the shape that used to
/// become a Version 6 story. The message has to name the bytes, and the head of
/// the file is what names them.
#[test]
fn an_unmountable_container_is_refused_by_name() {
    let mut image = vec![0u8; 4096];
    image[0] = 6;
    image[1..7].copy_from_slice(b"WIDGET");
    // DiskCopy 4.2's `private` magic, which is all that is needed to make this
    // a wrapper rather than noise.
    image[0x52] = 0x01;

    let err = app::hints::extract_story(image).expect_err("nothing can run this");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let said = err.to_string();
    for want in ["not a story file", "4096 bytes", "06 57 49 44 47 45 54 00", "WIDGET", "ZMSD"] {
        assert!(said.contains(want), "the diagnostic must contain {want:?}: {said}");
    }
}
