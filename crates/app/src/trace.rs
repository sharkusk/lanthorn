//! Multi-section debug trace (SQ-0403 follow-up). Best-effort, std-only.
//! Sections are toggled via `--trace <list>` and `/trace <list>`; output goes
//! to `<user_dir>/trace.log`, one `[section] message` line per event.

use std::io::Write as _;
use std::path::Path;

/// A trace section — the origin/kind of a traced event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// Story→interpreter display instructions (Glk calls / Z-machine screen opcodes).
    Screen,
    /// Automapper pipeline stages.
    Map,
    /// Host-side save/restore, VFS file I/O, input/events.
    HostIo,
    /// Per-turn v6 window/picture-canvas state snapshots (v6 stories only).
    V6,
}

impl Section {
    pub fn tag(self) -> &'static str {
        match self {
            Section::Screen => "screen",
            Section::Map => "map",
            Section::HostIo => "hostio",
            Section::V6 => "v6",
        }
    }
}

/// Which trace sections are active. Runtime state, set from `--trace`/`/trace`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct TraceSections {
    pub screen: bool,
    pub map: bool,
    pub hostio: bool,
    pub v6: bool,
}

impl TraceSections {
    pub fn any(self) -> bool {
        self.screen || self.map || self.hostio || self.v6
    }

    /// The active sections as a comma list (`"screen,map"`), or `"off"`.
    pub fn active_list(self) -> String {
        let mut v = Vec::new();
        if self.screen { v.push("screen"); }
        if self.map { v.push("map"); }
        if self.hostio { v.push("hostio"); }
        if self.v6 { v.push("v6"); }
        if v.is_empty() { "off".to_string() } else { v.join(",") }
    }

    /// Parse a comma-separated section list. `all`/`none` are keywords. Returns
    /// the resulting set plus any unrecognised tokens (for the caller to report).
    pub fn parse(s: &str) -> (TraceSections, Vec<String>) {
        let mut out = TraceSections::default();
        let mut unknown = Vec::new();
        for tok in s.split(',') {
            match tok.trim() {
                "" => {}
                "all" => out = TraceSections { screen: true, map: true, hostio: true, v6: true },
                "none" | "off" => out = TraceSections::default(),
                "screen" => out.screen = true,
                "map" => out.map = true,
                "hostio" => out.hostio = true,
                "v6" => out.v6 = true,
                other => unknown.push(other.to_string()),
            }
        }
        (out, unknown)
    }
}

fn log_path(user_dir: &Path) -> std::path::PathBuf {
    user_dir.join("trace.log")
}

/// Start a fresh trace log (best-effort). Used at boot so each run stands alone.
pub fn truncate(user_dir: &Path) {
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path(user_dir));
}

/// Append `lines` tagged with `section` (best-effort; a failed open is skipped).
pub fn write(user_dir: &Path, section: Section, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(user_dir))
    else {
        return;
    };
    // `[screen] ` = 9 chars; pad the tag so message columns align.
    let tag = format!("[{}]", section.tag());
    for line in lines {
        let _ = writeln!(f, "{tag:<8} {line}");
    }
}

/// Emit a single `hostio` line when the section is on (best-effort, no-op otherwise).
pub fn hostio(user_dir: &Path, on: bool, line: String) {
    if on {
        write(user_dir, Section::HostIo, &[line]);
    }
}

#[cfg(all(test, feature = "t-misc"))]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_comma_list_all_none_and_unknowns() {
        let (s, unknown) = TraceSections::parse("screen,map");
        assert!(s.screen && s.map && !s.hostio && !s.v6);
        assert!(unknown.is_empty());

        let (s, _) = TraceSections::parse("all");
        assert!(s.screen && s.map && s.hostio && s.v6);

        let (s, _) = TraceSections::parse("none");
        assert!(!s.any());

        let (s, unknown) = TraceSections::parse(" screen , bogus ");
        assert!(s.screen && !s.map);
        assert_eq!(unknown, vec!["bogus".to_string()]);

        let (s, _) = TraceSections::parse("v6");
        assert!(s.v6 && !s.screen);

        assert_eq!(TraceSections::default().active_list(), "off");
        assert_eq!(
            TraceSections { screen: true, map: true, hostio: false, v6: false }.active_list(),
            "screen,map"
        );
    }

    #[test]
    fn write_tags_and_aligns_truncate_resets() {
        let dir = std::env::temp_dir().join(format!("lanthorn-trace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("trace.log");
        let _ = std::fs::remove_file(&log);

        write(&dir, Section::Screen, &["@split_window(1)".to_string()]);
        write(&dir, Section::Map, &["detect chains".to_string()]);
        let body = std::fs::read_to_string(&log).unwrap();
        assert!(body.contains("[screen] @split_window(1)"));
        assert!(body.contains("[map]    detect chains"), "map tag padded to width 8: {body:?}");

        truncate(&dir);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "", "truncate starts fresh");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hostio_writes_only_when_on() {
        let dir = std::env::temp_dir().join(format!("bm-hostio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        hostio(&dir, false, "save_state(auto, 1024 bytes)".to_string());
        assert!(!dir.join("trace.log").exists() || std::fs::read_to_string(dir.join("trace.log")).unwrap().is_empty());
        hostio(&dir, true, "save_state(auto, 1024 bytes)".to_string());
        assert!(std::fs::read_to_string(dir.join("trace.log")).unwrap().contains("[hostio] save_state(auto, 1024 bytes)"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
