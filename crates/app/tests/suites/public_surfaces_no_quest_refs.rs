//! SQ-1509: quest numbers (`SQ-xxxx`) mean nothing to an end user, so they must
//! never appear on a surface a player reads. This scans the surfaces that
//! matter and fails the build the moment one creeps back in.
//!
//! Covered: `README.md`, `CHANGELOG.md`, everything under `docs/guide/` and
//! `docs/reference/`, every `crates/*/README.md`, and the player-visible
//! comment strings [`app::config_template`] writes into a seeded
//! `~/.lanthorn/config.toml` (the `Row` comment arrays — NOT the file's own
//! Rust doc comments, which are developer-only and exempt).
//!
//! `docs/internals/`, code comments elsewhere, test names and commit history
//! are deliberately out of scope — see `CLAUDE.md`.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let raw = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    raw.canonicalize().unwrap_or(raw)
}

/// A line number (1-based) and the offending text, for one file.
struct Hit {
    file: PathBuf,
    line: usize,
    text: String,
}

/// Byte offset of the first `SQ-####` (exactly four ASCII digits) in `s`, if any.
fn find_sq_ref(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if &bytes[i..i + 3] == b"SQ-" && bytes[i + 3..i + 7].iter().all(|b| b.is_ascii_digit()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn scan_plain_file(path: &PathBuf, hits: &mut Vec<Hit>) {
    let Ok(src) = std::fs::read_to_string(path) else { return };
    for (i, line) in src.lines().enumerate() {
        if find_sq_ref(line).is_some() {
            hits.push(Hit { file: path.clone(), line: i + 1, text: line.trim().to_string() });
        }
    }
}

fn markdown_files_under(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files_under(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// The player-visible comment strings in `config_template.rs`: text inside a
/// quoted string literal on a line that is not itself a `//`/`///`/`//!`
/// comment. A `//` trailing a real code line (there are none in this file, per
/// its own no-`://` content) would also be stripped; that is deliberately
/// conservative — this scan only needs to catch a live `Row` string.
fn scan_config_template_strings(path: &PathBuf, hits: &mut Vec<Hit>) {
    let Ok(src) = std::fs::read_to_string(path) else { return };
    for (i, raw_line) in src.lines().enumerate() {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("//") {
            continue; // `//`, `///` and `//!` are developer-only here.
        }
        let code = match raw_line.find("//") {
            Some(at) => &raw_line[..at],
            None => raw_line,
        };
        if find_sq_ref(code).is_some() {
            hits.push(Hit { file: path.clone(), line: i + 1, text: code.trim().to_string() });
        }
    }
}

#[test]
fn public_surfaces_carry_no_quest_numbers() {
    let root = workspace_root();
    let mut hits = Vec::new();

    scan_plain_file(&root.join("README.md"), &mut hits);
    scan_plain_file(&root.join("CHANGELOG.md"), &mut hits);

    let mut guide_and_reference = Vec::new();
    markdown_files_under(&root.join("docs").join("guide"), &mut guide_and_reference);
    markdown_files_under(&root.join("docs").join("reference"), &mut guide_and_reference);
    for path in &guide_and_reference {
        scan_plain_file(path, &mut hits);
    }

    if let Ok(crates_dir) = std::fs::read_dir(root.join("crates")) {
        for entry in crates_dir.flatten() {
            let readme = entry.path().join("README.md");
            if readme.is_file() {
                scan_plain_file(&readme, &mut hits);
            }
        }
    }

    scan_config_template_strings(&root.join("crates").join("app").join("src").join("config_template.rs"), &mut hits);

    assert!(
        hits.is_empty(),
        "quest number(s) leaked onto a public-facing surface (see SQ-1509):\n{}",
        hits.iter()
            .map(|h| format!("{}:{}: {}", h.file.strip_prefix(&root).unwrap_or(&h.file).display(), h.line, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn find_sq_ref_matches_only_a_real_quest_number() {
    assert_eq!(find_sq_ref("nothing here"), None);
    assert_eq!(find_sq_ref("SQ-123 too short"), None);
    assert_eq!(find_sq_ref("SQ-12345 still matches the first four digits"), Some(0));
    assert_eq!(find_sq_ref("see (SQ-1509) here"), Some(5));
}
