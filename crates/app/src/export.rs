use std::io;
use std::path::{Path, PathBuf};

/// Resolve an export destination the SQ-0284 way: no dest → `game_dir/<default_name>`;
/// a bare name (no separator) → `game_dir/<name>` with the default's extension appended
/// if the name has none; a value containing a path separator (or absolute) → verbatim.
pub fn resolve_export_path(dest: Option<&str>, game_dir: &Path, default_name: &str) -> PathBuf {
    match dest.map(str::trim) {
        None | Some("") => game_dir.join(default_name),
        Some(d) if d.contains('/') || d.contains('\\') => PathBuf::from(d),
        Some(d) => {
            let name = if Path::new(d).extension().is_some() {
                d.to_string()
            } else if let Some(ext) = Path::new(default_name).extension().and_then(|e| e.to_str()) {
                format!("{d}.{ext}")
            } else {
                d.to_string()
            };
            game_dir.join(name)
        }
    }
}

/// Write `lines` to a file, resolving the destination via [`resolve_export_path`]
/// against `game_dir` with the default name `transcript.txt`.
///
/// Parent directories are created if missing.
/// Returns the path that was written.
pub fn export_transcript(
    lines: &[String],
    dest: Option<&str>,
    game_dir: &Path,
) -> io::Result<PathBuf> {
    let target = resolve_export_path(dest, game_dir, "transcript.txt");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = format!("{}\n", lines.join("\n"));
    crate::storage::atomic_write(&target, content.as_bytes())?;
    Ok(target)
}

/// The `/dump-windows` log, under the lanthorn home: `<user_dir>/dump-windows.log`.
pub fn window_dump_path(user_dir: &Path) -> PathBuf {
    user_dir.join("dump-windows.log")
}

/// Append one `/dump-windows` capture to that log, stamped with the instant it was
/// taken, and return the path (SQ-0756).
///
/// The dump is drawn into the terminal, and selecting it there also selects the kitty
/// unicode placeholder glyphs the v6 pane is made of — the user's paste came back
/// dense with placeholders and truncated mid-field, corrupted by the very protocol
/// the dump exists to diagnose. A file is the same text with nothing composited over
/// it, readable from another terminal while the TUI is still running, and it outlives
/// the session.
///
/// APPENDS rather than replaces: an investigation takes several dumps — before a
/// move, after it — and each is evidence about a different frame.
pub fn append_window_dump(user_dir: &Path, lines: &[String]) -> io::Result<PathBuf> {
    use std::io::Write;
    let target = window_dump_path(user_dir);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&target)?;
    writeln!(f, "=== /dump-windows {} ===", jiff::Timestamp::now())?;
    for line in lines {
        writeln!(f, "{line}")?;
    }
    writeln!(f)?;
    Ok(target)
}

/// The `/dump-terminal` log, under the lanthorn home: `<user_dir>/dump-terminal.log`.
pub fn terminal_dump_path(user_dir: &Path) -> PathBuf {
    user_dir.join("dump-terminal.log")
}

/// Append one `/dump-terminal` report to that log, stamped, and return the path
/// (SQ-0994).
///
/// Its own file rather than a section of the window dump: this one describes the
/// TERMINAL and the traffic it is being sent, which is what somebody attaches to
/// a bug report, and burying it under a hundred lines of window geometry would
/// make it harder to find, not easier. Appends for the reason
/// [`append_window_dump`] does — the interesting comparison is one report against
/// another taken a few frames later.
pub fn append_terminal_dump(user_dir: &Path, lines: &[String]) -> io::Result<PathBuf> {
    use std::io::Write;
    let target = terminal_dump_path(user_dir);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&target)?;
    writeln!(f, "=== /dump-terminal {} ===", jiff::Timestamp::now())?;
    for line in lines {
        writeln!(f, "{line}")?;
    }
    writeln!(f)?;
    Ok(target)
}

/// The `/dump-cells` log, under the lanthorn home: `<user_dir>/dump-cells.log`.
pub fn cell_dump_path(user_dir: &Path) -> PathBuf {
    user_dir.join("dump-cells.log")
}

/// Append one `/dump-cells` capture to that log, stamped, and return the path
/// (SQ-0761).
///
/// A separate file from the window dump on purpose: this one is a hundred-odd lines
/// of fixed-width grid per capture, and interleaving it with the geometry dump would
/// bury both. Appends for the same reason `append_window_dump` does — an
/// investigation takes a dump before a move and another after it, and the pair is
/// the evidence.
pub fn append_cell_dump(user_dir: &Path, lines: &[String]) -> io::Result<PathBuf> {
    use std::io::Write;
    let target = cell_dump_path(user_dir);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&target)?;
    writeln!(f, "=== /dump-cells {} ===", jiff::Timestamp::now())?;
    for line in lines {
        writeln!(f, "{line}")?;
    }
    writeln!(f)?;
    Ok(target)
}

#[cfg(all(test, feature = "t-state"))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn resolve_none_uses_default_name_in_game_dir() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(None, gd, "map.svg"), PathBuf::from("/data/Zork1.z5/map.svg"));
    }
    #[test]
    fn resolve_bare_name_appends_default_ext_when_missing() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("before"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.svg"));
        // an explicit extension on the bare name is preserved
        assert_eq!(resolve_export_path(Some("before.dot"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.dot"));
    }
    #[test]
    fn resolve_path_bearing_value_is_verbatim() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("/tmp/x.svg"), gd, "map.svg"), PathBuf::from("/tmp/x.svg"));
    }

    #[test]
    fn export_transcript_resolves_dest_and_writes() {
        let dir = std::env::temp_dir().join(format!("lanthorn-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lines = vec!["a".to_string(), "b".to_string()];
        let p1 = export_transcript(&lines, None, &dir).unwrap();
        assert_eq!(p1, dir.join("transcript.txt"));
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), "a\nb\n");
        let p2 = export_transcript(&lines, Some("out.txt"), &dir).unwrap();
        assert_eq!(p2, dir.join("out.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
