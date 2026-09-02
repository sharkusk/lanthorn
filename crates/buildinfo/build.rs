//! Compute the long version string at build time and expose it as an env var
//! (`LANTHORN_LONG_VERSION`) that `src/lib.rs` reads with `env!`.
//!
//! - On an exact release tag → the clean crate version (`0.1.0`).
//! - Otherwise → `0.1.0 (<short-hash>)`, plus `-dirty` when the working tree has
//!   uncommitted changes.
//! - With no git available (a source tarball) → the plain crate version.

use std::process::Command;

/// Run `git <args>`; return trimmed stdout, or `None` on failure/empty output.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn main() {
    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();

    let long = if git(&["describe", "--tags", "--exact-match", "HEAD"]).is_some() {
        base.clone()
    } else if let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) {
        // `git status --porcelain` prints nothing on a clean tree, so a non-empty
        // result (git() returns Some) means there are uncommitted changes.
        // Untracked files are not changes — the tree routinely carries scratch
        // files and gitignored fixtures, and `git describe --dirty` ignores them
        // too — so only tracked modifications earn the suffix.
        let suffix = if git(&["status", "--porcelain", "--untracked-files=no"]).is_some() {
            "-dirty"
        } else {
            ""
        };
        format!("{base} ({hash}{suffix})")
    } else {
        base.clone()
    };
    println!("cargo:rustc-env=LANTHORN_LONG_VERSION={long}");

    // Rebuild when HEAD or the checked-out ref moves, so the hash stays current
    // after a commit or checkout. (Working-tree edits do not retrigger, so the
    // `-dirty` marker reflects the tree at the last build of this crate.)
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/packed-refs");
        if let Ok(head) = std::fs::read_to_string(format!("{git_dir}/HEAD")) {
            if let Some(r) = head.trim().strip_prefix("ref: ") {
                println!("cargo:rerun-if-changed={git_dir}/{r}");
            }
        }
    }
}
