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

/// Load a story fixture by filename.  Returns `None` if the file does not
/// exist (so callers can early-return / skip rather than fail).
pub fn load(name: &str) -> Option<Vec<u8>> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests");
    path.push("fixtures");
    path.push(name);
    std::fs::read(&path).ok()
}
