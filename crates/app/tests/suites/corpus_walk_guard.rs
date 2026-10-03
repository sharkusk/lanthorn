//! The shared corpus walker follows symlinks but cannot loop (SQ-1708).
//!
//! A `stories/stories -> stories` self-link once made every recursive corpus walk
//! recurse ~32 levels to ELOOP and report each file 32 times. `files_under` keeps a
//! canonical-path visited set instead; these cases pin both halves of that — it
//! terminates, and it still follows a link that goes somewhere real.

#![cfg(unix)]

use crate::fixture_paths::files_under;
use std::path::PathBuf;

fn walk(root: &std::path::Path) -> Vec<String> {
    let mut out: Vec<PathBuf> = Vec::new();
    files_under(root, &mut out);
    // Non-vacuity / dedupe: no canonical path may appear twice.
    let mut canon: Vec<PathBuf> = out.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect();
    canon.sort();
    let n = canon.len();
    canon.dedup();
    assert_eq!(n, canon.len(), "a file was reported twice: {out:?}");
    let mut names: Vec<String> =
        out.iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn a_self_loop_terminates_with_exactly_the_real_files() {
    let root = app::scratch_dir("corpus-loop");
    std::fs::write(root.join("a.z5"), b"a").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("b.z5"), b"b").unwrap();
    std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
    std::os::unix::fs::symlink(&root, root.join("sub").join("up")).unwrap();
    assert_eq!(walk(&root), ["a.z5", "sub/b.z5"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_link_to_a_directory_outside_the_root_is_followed() {
    let root = app::scratch_dir("corpus-in");
    let outside = app::scratch_dir("corpus-out");
    std::fs::write(root.join("a.z5"), b"a").unwrap();
    std::fs::write(outside.join("far.z5"), b"f").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("elsewhere")).unwrap();
    assert_eq!(walk(&root), ["a.z5", "elsewhere/far.z5"]);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn two_links_to_one_directory_yield_its_files_once() {
    let root = app::scratch_dir("corpus-dup");
    let outside = app::scratch_dir("corpus-dup-out");
    std::fs::write(outside.join("far.z5"), b"f").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("one")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("two")).unwrap();
    assert_eq!(walk(&root).len(), 1);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}
