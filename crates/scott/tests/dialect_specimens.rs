//! Re-runs [`scott::detect_dialect`]'s specimen measurement over the real
//! game files, when they are present (SQ-1414).
//!
//! `detect_dialect`'s own doc states what each signature is and which files
//! it was measured on. Those files are commercial game images and are not
//! redistributable, so none is committed and CI never sees one — the
//! hand-built fixtures in `loader.rs`'s own test module carry the format
//! rules there. This suite is what keeps the doc's *numbers* honest: it
//! asserts the counts and offsets that doc quotes, against the actual
//! corpus, and **skips vacuously** with an explanation when the corpus is
//! absent.
//!
//! # Getting the corpus
//!
//! Point `SCOTT_DIALECT_FIXTURES` at a directory, or create
//! `stories/scott-dialects/`, holding the two IF Archive downloads
//! unpacked into subdirectories named `ti99/` and `spectrum/`:
//!
//! ```text
//! <fixtures>/ti99/adv01.fiad … adv12.fiad
//!     https://ifarchive.org/if-archive/scott-adams/games/ti99/scott_adams_ti99_games.zip
//! <fixtures>/spectrum/m1goldba.z80 … temple.z80
//!     https://ifarchive.org/if-archive/games/spectrum/mystsoft.zip
//! ```
//!
//! A vacuous skip reads exactly like a pass, so the skip below prints WHY
//! it skipped and what to do about it, rather than returning silently.

use std::path::PathBuf;

use scott::{detect_dialect, Dialect};

/// The fixture directory, if one is configured and exists.
fn fixtures() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.is_dir())
}

/// Every regular file directly inside `<fixtures>/<sub>`, sorted by name so
/// a failure names the same file every run.
fn specimens(sub: &str) -> Vec<(String, Vec<u8>)> {
    let Some(dir) = fixtures().map(|d| d.join(sub)) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Vec<u8>)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // Skip the archive itself and any editor/OS droppings.
            if name.starts_with('.') || name.ends_with(".zip") {
                return None;
            }
            std::fs::read(e.path()).ok().map(|b| (name, b))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn skip(what: &str) {
    eprintln!(
        "SKIP: no {what} specimens — set SCOTT_DIALECT_FIXTURES or populate \
         stories/scott-dialects/ (see this suite's module doc for the two \
         IF Archive URLs). This is a vacuous skip, NOT a pass."
    );
}

#[test]
fn every_ti994a_release_is_detected_at_the_documented_offset() {
    let files = specimens("ti99");
    if files.is_empty() {
        skip("TI-99/4A");
        return;
    }
    const MARKER: &[u8] = b"\x30\x30\x30\x30\x00\x30\x30\x00\x28\x28";
    for (name, bytes) in &files {
        assert_eq!(
            detect_dialect(bytes),
            Some(Dialect::Ti994aBytecode),
            "{name} ({} bytes) is a TI-99/4A image and must be named as one",
            bytes.len()
        );
        // `detect_dialect`'s doc quotes offset 1417 across all twelve
        // releases, and calls that a measurement rather than a rule. If a
        // future specimen puts it elsewhere the doc is what needs editing,
        // so fail here loudly rather than relaxing the check.
        let at = bytes
            .windows(MARKER.len())
            .position(|w| w == MARKER)
            .expect("marker present, since detect_dialect found it");
        assert_eq!(at, 1417, "{name}: marker offset moved; update the doc");
        let occurrences = bytes.windows(MARKER.len()).filter(|w| *w == MARKER).count();
        assert_eq!(occurrences, 1, "{name}: marker is meant to be unique");
    }
    assert_eq!(files.len(), 12, "the documented corpus is Adventures 1-12");
}

#[test]
fn the_spectrum_corpus_splits_exactly_as_documented() {
    let files = specimens("spectrum");
    if files.is_empty() {
        skip("ZX Spectrum");
        return;
    }
    // The names `detect_dialect`'s doc commits to, by the answer it gives.
    let packed = ["seablood.z80", "sherwood.z80"];
    let undetected = [
        "blizzard.z80",
        "heman.z80",
        "kayleth.z80",
        "rbplanet.z80",
        "temple.z80",
    ];
    let (mut unpacked_n, mut packed_n, mut none_n) = (0, 0, 0);
    for (name, bytes) in &files {
        let got = detect_dialect(bytes);
        let want = if packed.contains(&name.as_str()) {
            Some(Dialect::CompressedActionTable)
        } else if undetected.contains(&name.as_str()) {
            None
        } else {
            Some(Dialect::C64OrZxSnapshot)
        };
        assert_eq!(got, want, "{name}");
        match got {
            Some(Dialect::CompressedActionTable) => packed_n += 1,
            Some(_) => unpacked_n += 1,
            None => none_n += 1,
        }
    }
    assert_eq!(
        (unpacked_n, packed_n, none_n),
        (13, 2, 5),
        "the documented 13/2/5 split over 20 snapshots"
    );
}

#[test]
fn no_spectrum_specimen_is_mistaken_for_a_loadable_dat() {
    let files = specimens("spectrum");
    if files.is_empty() {
        skip("spectrum");
        return;
    }
    for (name, bytes) in &files {
        // A memory image this crate cannot yet read must never parse as a
        // ScottFree `.dat`: that would be a wrong game, not a refused one.
        // (The TI-99/4A specimens are a different case since SQ-1414 — they
        // are read, by `parse_ti994a`, and `ti994a_specimens.rs` is the
        // suite that checks what comes out.)
        assert!(
            scott::Database::parse(bytes).is_err(),
            "{name} parsed as a .dat"
        );
    }
}
