//! Test fixture loader — reads story files from `crates/zvm/tests/fixtures/`.
//!
//! Returns `None` if the fixture is absent so that fixture-backed tests can
//! skip cleanly rather than failing.
//!
//! Gated behind the `fixtures` Cargo feature and not part of this crate's
//! normal public surface: [`load`] bakes the build machine's absolute
//! `CARGO_MANIFEST_DIR` into any binary that links it, which is fine for this
//! crate's own test binaries and wrong for anything a downstream embedder
//! ships.

use std::path::PathBuf;

/// Resolve a story fixture by filename to an EXISTING path. Returns `None` if
/// neither this crate's own `tests/fixtures/` nor (for `minizork.z3` only,
/// see below) its one fallback location has it — never a non-existent path,
/// unlike the app crate's `fixture_path`, since a caller here (`dfrotz_probe`
/// in `save_interop.rs`) hands the result to an external process that cannot
/// itself fall back.
pub fn path(name: &str) -> Option<PathBuf> {
    let mut primary = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    primary.push("tests");
    primary.push("fixtures");
    primary.push(name);
    if primary.is_file() {
        return Some(primary);
    }

    // `minizork.z3` moved to the fetched fixture set (SQ-1453): this crate
    // takes zero external dependencies, so it has no fetch machinery of its
    // own — `scripts/fetch-fixtures.sh` (driven from the app crate) populates
    // `crates/app/tests/fixtures/stories/minizork-r34-s871124.z3` instead,
    // the same bytes under the `stories/`-era filename several `app` suites
    // already ask for through `fixture_path`. Fall back to that one location
    // for that one name rather than building a general multi-directory
    // search this crate does not otherwise need.
    if name == "minizork.z3" {
        let alt = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../app/tests/fixtures/stories/minizork-r34-s871124.z3");
        if alt.is_file() {
            return Some(alt);
        }
    }

    None
}

/// Load a story fixture by filename.  Returns `None` if the file does not
/// exist (so callers can early-return / skip rather than fail).
pub fn load(name: &str) -> Option<Vec<u8>> {
    std::fs::read(path(name)?).ok()
}
