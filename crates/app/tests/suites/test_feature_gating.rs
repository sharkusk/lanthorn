//! One rule, checked mechanically, about `app`'s ~3,271 in-crate `#[test]`s (SQ-1242):
//! every `#[cfg(test)] mod` under `crates/app/src/` names one of the `t-*` group
//! features declared in `crates/app/Cargo.toml`, so `cargo check -p app --lib --tests`
//! with no features on compiles none of them (33s, against 5m19s unguarded) and CI's
//! `--all-features` run still compiles every one.
//!
//! A bare `#[cfg(test)]` on a `mod` item is invisible to that plan: it is `true`
//! whenever `--tests` is, features or not, so one file left ungated silently drags the
//! whole point back to where SQ-1242 started. The next author adding a source file has
//! no reason to know this convention exists, which is exactly the shape
//! `palette_lock_discipline` and `scratch_path_discipline` guard elsewhere — a
//! hand-maintained invariant across ~150 `mod tests` blocks needs a source-level case,
//! not a comment.
//!
//! # The three things checked
//!
//! | case | fails when |
//! |---|---|
//! | [`every_test_module_is_feature_gated`] | a `mod` item under `crates/app/src/` carries bare `#[cfg(test)]` instead of `#[cfg(all(test, feature = "t-..."))]` |
//! | [`every_named_feature_is_declared`] | a `feature = "t-..."` string in `src/` names something `Cargo.toml`'s `[features]` table does not declare (a typo would otherwise silently gate a module to a feature nothing ever turns on) |
//! | [`t_all_lists_every_group`] | `t-all`'s member list and the set of `t-*` features declared beside it disagree in either direction |
//!
//! Falsified by reverting any one file's rewrite back to a bare `#[cfg(test)]` on its
//! `mod tests` — done by hand during SQ-1242's own verification, not kept as a test
//! here, since a suite that un-gates its own fixture on purpose is exactly the kind of
//! thing the growing pile of `scratch_*.rs` litter this repo's hygiene rules warn about.
//!
//! Non-module `#[cfg(test)]` items (helper fns, `use`s, `impl` blocks) are out of scope
//! here by design — SQ-1242's brief leaves most of those alone deliberately, gating only
//! the ones that would otherwise dead-code under a single group. This file cannot tell
//! "deliberately shared test support" from "forgotten"; that judgment call is a build
//! warning's job (`cargo check -p app --lib --tests --features t-<group>`), not this
//! scan's.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `.rs` file under `crates/app/src/`, as (path relative to the workspace, source).
fn app_src_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let src = root.join("crates").join("app").join("src");
    let mut out = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push((relative(&root, &path), text));
                }
            }
        }
    }
    out.sort();
    out
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// A line that is, after trimming, exactly `#[cfg(test)]` — the bare form. The correctly
/// migrated form is `#[cfg(all(test, feature = "t-..."))]`, which does not match this.
fn is_bare_cfg_test_line(line: &str) -> bool {
    line.trim() == "#[cfg(test)]"
}

/// True when `line`, once trimmed, opens a `mod` item: `mod foo {`, `pub mod foo {`,
/// `pub(crate) mod foo;`, etc. Good enough for this scan because every real case in this
/// crate is one attribute followed immediately (blank/attribute lines aside) by a `mod`
/// keyword at the start of a line — nothing here is nested inside an expression.
fn opens_mod_item(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix("pub(crate)").map(str::trim_start).unwrap_or(t);
    let t = t.strip_prefix("pub").map(str::trim_start).unwrap_or(t);
    t.starts_with("mod ") && (t.contains('{') || t.trim_end().ends_with(';'))
}

/// For a `#[cfg(test)]` line at index `i`, find the next line that is neither blank nor
/// another attribute (`#[...]`) — skipping doc comments would be needed too, but no
/// `mod tests` in this crate carries one between its cfg and itself.
fn next_substantive_line<'a>(lines: &'a [&'a str], i: usize) -> Option<&'a str> {
    let mut j = i + 1;
    while j < lines.len() {
        let t = lines[j].trim();
        if t.is_empty() || t.starts_with("#[") {
            j += 1;
            continue;
        }
        return Some(lines[j]);
    }
    None
}

#[test]
fn every_test_module_is_feature_gated() {
    let mut offenders = Vec::new();
    for (path, src) in app_src_sources() {
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !is_bare_cfg_test_line(line) {
                continue;
            }
            if let Some(next) = next_substantive_line(&lines, i) {
                if opens_mod_item(next) {
                    offenders.push(format!("{path}:{}", i + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these mod items carry a bare #[cfg(test)] instead of \
         #[cfg(all(test, feature = \"t-<group>\"))]:\n\x20   {}\n\
         SQ-1242 gated every in-crate test module behind a t-* Cargo feature so that \
         `cargo check -p app --lib --tests` with no features on compiles none of them. A \
         bare #[cfg(test)] on a mod item is `true` whenever --tests is, features or not, \
         so it silently opts back into being compiled on every check regardless of which \
         group (if any) is enabled. Pick the group matching the file's module-tree area \
         (crates/app/Cargo.toml's [features] table lists all nine with what each covers) \
         and gate it the same way its neighbours in the file already are.",
        offenders.join("\n\x20   ")
    );
}

/// Parse `crates/app/Cargo.toml`'s `[features]` table into (name, member-list) pairs.
/// Member lists are only meaningful for `t-all`; every other `t-*` feature is `[]`.
fn declared_features() -> Vec<(String, Vec<String>)> {
    let root = workspace_root();
    let toml_path = root.join("crates").join("app").join("Cargo.toml");
    let text = std::fs::read_to_string(&toml_path).expect("crates/app/Cargo.toml is readable");
    let mut out = Vec::new();
    let mut in_features = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_features = t == "[features]";
            continue;
        }
        if !in_features || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = t.split_once('=') else { continue };
        let name = name.trim().to_string();
        let rest = rest.trim();
        let members: Vec<String> = if let Some(inner) = rest.strip_prefix('[') {
            let inner = inner.split(']').next().unwrap_or("");
            inner
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        out.push((name, members));
    }
    out
}

#[test]
fn every_named_feature_is_declared() {
    let declared: Vec<String> = declared_features().into_iter().map(|(n, _)| n).collect();
    let mut offenders = Vec::new();
    for (path, src) in app_src_sources() {
        for (i, line) in src.lines().enumerate() {
            let mut rest = line;
            while let Some(at) = rest.find("feature = \"") {
                let after = &rest[at + "feature = \"".len()..];
                let Some(end) = after.find('"') else { break };
                let name = &after[..end];
                if name.starts_with("t-") && !declared.iter().any(|d| d == name) {
                    offenders.push(format!("{path}:{} names undeclared feature \"{name}\"", i + 1));
                }
                rest = &after[end + 1..];
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these cfg attributes name a t-* feature Cargo.toml does not declare:\n\x20   {}\n\
         A typo here gates a module to a feature nothing ever turns on, which means it is \
         never compiled by any single-group check AND never run by t-all/--all-features — \
         silently, since cargo does not warn about a feature predicate that is always \
         false. Match the spelling in crates/app/Cargo.toml's [features] table exactly.",
        offenders.join("\n\x20   ")
    );
}

#[test]
fn t_all_lists_every_group() {
    let declared = declared_features();
    let all_groups: Vec<&str> = declared
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| n.starts_with("t-") && *n != "t-all")
        .collect();
    let t_all_members: Vec<String> = declared
        .iter()
        .find(|(n, _)| n == "t-all")
        .map(|(_, members)| members.clone())
        .unwrap_or_default();

    let missing: Vec<&str> =
        all_groups.iter().filter(|g| !t_all_members.iter().any(|m| m == *g)).copied().collect();
    let extra: Vec<&String> =
        t_all_members.iter().filter(|m| !all_groups.contains(&m.as_str())).collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "t-all's member list and the declared t-* group features disagree: \
         missing from t-all: {missing:?}, listed in t-all but not declared as its own \
         feature: {extra:?}.\n\
         t-all exists so `--features t-all` (and CI's --all-features) compiles every \
         group's tests; a group left out of it is silently skipped by both.",
    );
}
