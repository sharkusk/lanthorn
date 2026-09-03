//! SQ-1222: `docs/reference/*.md` is GENERATED from the app's own registries
//! (`app::docs_reference`), not hand-maintained — so a command, key binding,
//! config setting or style selector can never be documented differently from
//! what the code actually does.
//!
//! Four cases, one per generated file, all following the same shape: render
//! the table from the live registry and compare it against the committed
//! file. Set `LANTHORN_REGEN_DOCS=1` to have the case WRITE the file instead
//! of asserting — that is how the four files are (re)generated:
//!
//! ```sh
//! LANTHORN_REGEN_DOCS=1 cargo nextest run -p lanthorn docs_reference
//! ```
//!
//! A fifth case, [`every_relative_markdown_link_resolves`], walks every
//! Markdown file this branch ships and checks every relative link target
//! exists on disk.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let raw = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    // Canonicalize once, here: every other path in this file is built by
    // joining onto this one and then lexically normalizing (`normalize`), and
    // a `..`-laden root would make that normalization inconsistent between a
    // path that started from a normalized root and one that did not.
    raw.canonicalize().unwrap_or(raw)
}

fn reference_path(name: &str) -> PathBuf {
    workspace_root().join("docs").join("reference").join(name)
}

/// Compare `rendered` against the committed file, or write it when
/// `LANTHORN_REGEN_DOCS=1` is set.
fn check_or_regen(name: &str, rendered: String) {
    let path = reference_path(name);
    if std::env::var("LANTHORN_REGEN_DOCS").as_deref() == Ok("1") {
        std::fs::write(&path, &rendered).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e} — has it been generated yet?", path.display()));
    assert_eq!(
        committed, rendered,
        "docs/reference/{name} is stale; run LANTHORN_REGEN_DOCS=1 cargo nextest run -p lanthorn docs_reference to regenerate"
    );
}

#[test]
fn commands_md_matches_the_slash_registry() {
    check_or_regen("commands.md", app::docs_reference::render_commands());
}

#[test]
fn keys_md_matches_the_default_keymap() {
    check_or_regen("keys.md", app::docs_reference::render_keys());
}

#[test]
fn config_md_matches_the_config_template() {
    check_or_regen("config.md", app::docs_reference::render_config());
}

#[test]
fn style_md_matches_the_theme_registry() {
    check_or_regen("style.md", app::docs_reference::render_style());
}

// ── every_relative_markdown_link_resolves ──────────────────────────────────

/// Every Markdown file this case checks: `README.md` plus everything under
/// `docs/`, EXCLUDING `docs/plans/`, `docs/superpowers/` and `docs/design/` —
/// those are working notes, not living docs, and are not held to this bar.
fn markdown_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut out = vec![root.join("README.md")];
    let docs = root.join("docs");
    let mut stack = vec![docs.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "plans" | "superpowers" | "design") && path.parent() == Some(docs.as_path()) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    out
}

/// Every `](target)` and `]: target` link target in `src`, paired with the
/// 1-based line it starts on.
///
/// Backtick code spans are masked out before scanning: `plundered_hearts[infocom_1987](r26)(!).g64`
/// in `docs/internals/architecture.md` is a filename inside inline code, not a
/// link, and reads exactly like one to a naive scanner — bracket then paren,
/// no different from a real `[text](target)`. A real broken link inside an
/// *un-fenced* example is still a broken link on the page as rendered, so only
/// single-backtick spans are masked; fenced ``` blocks are not.
fn link_targets(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let masked = mask_code_spans(line);
        let bytes = masked.as_bytes();
        let mut j = 0;
        while j < bytes.len() {
            // Inline form: ](target)
            if bytes[j] == b']' && j + 1 < bytes.len() && bytes[j + 1] == b'(' {
                let start = j + 2;
                if let Some(end_rel) = masked[start..].find(')') {
                    out.push((i + 1, masked[start..start + end_rel].to_string()));
                    j = start + end_rel;
                    continue;
                }
            }
            j += 1;
        }
        // Reference form: [label]: target (only at line start, optionally indented).
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            if let Some(colon) = trimmed.find("]: ") {
                let target = trimmed[colon + 3..].split_whitespace().next().unwrap_or("");
                if !target.is_empty() {
                    out.push((i + 1, target.to_string()));
                }
            }
        }
    }
    out
}

/// Replace every character inside a backtick-delimited code span with `x`,
/// leaving the backticks and everything outside a span untouched. The caller
/// scans and slices the RESULT of this function only, so it never compares a
/// byte offset into the masked string against one into the original line.
fn mask_code_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_span = false;
    for ch in line.chars() {
        if ch == '`' {
            in_span = !in_span;
            out.push(ch);
        } else if in_span {
            out.push('x');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Whether `target` is out of scope for a same-repo existence check.
fn is_external(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with('#')
        || target.is_empty()
}

#[test]
fn every_relative_markdown_link_resolves() {
    let root = workspace_root();
    let mut broken: Vec<String> = Vec::new();
    for path in markdown_files() {
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        let dir = path.parent().unwrap_or(&root).to_path_buf();
        for (line, target) in link_targets(&src) {
            if is_external(&target) {
                continue;
            }
            let target_no_fragment = target.split('#').next().unwrap_or(&target);
            if target_no_fragment.is_empty() {
                continue;
            }
            let resolved = if let Some(stripped) = target_no_fragment.strip_prefix('/') {
                root.join(stripped)
            } else {
                normalize(&dir.join(target_no_fragment))
            };
            if resolved.exists() {
                continue;
            }
            broken.push(format!(
                "{}:{line} -> {target}",
                path.strip_prefix(&root).unwrap_or(&path).display()
            ));
        }
    }
    assert!(broken.is_empty(), "broken relative markdown link(s):\n{}", broken.join("\n"));
}

/// Lexically collapse `.`/`..` components without touching the filesystem —
/// `Path::canonicalize` requires the path to exist, and a broken link's whole
/// point is that it does not.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
