//! `lanthorn --migrate-user-dir`: move a legacy `~/.lanthorn` into the platform
//! folders (SQ-1725).
//!
//! The legacy folder stays supported (`user_dirs`), so this is opt-in. [`plan`] and
//! [`execute`] work over explicit paths, so tests drive them in scratch folders and
//! never touch a real home; [`run_cli`] is the one function that reads the real
//! environment.
//!
//! Mapping, by top-level entry of the legacy folder: `config.toml` and
//! `style.toml` go to the config root; the children of `cache/` go straight into
//! the cache root; everything else goes to the data root. OS clutter is deleted.
//! Nothing is merged: if any destination exists, the plan is refused whole.
//! Entries are only renamed; nothing is copied or recursively deleted, so a
//! rename the OS refuses (another disk) is reported for the person to do by hand.

use crate::user_dirs::{home_dir, Inputs, UserDirs, LEGACY_DIR};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Files the OS scatters in a folder; deleted rather than moved.
const CLUTTER: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];

/// One entry to move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// What [`execute`] will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub legacy: PathBuf,
    pub moves: Vec<Move>,
    pub clutter: Vec<PathBuf>,
    /// Folders to remove once empty: the old `cache/`, then the legacy folder.
    pub cleanup: Vec<PathBuf>,
}

/// Why no plan could be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The legacy folder does not exist.
    NoLegacy(PathBuf),
    /// Destinations that already exist; nothing was changed.
    Conflicts(Vec<PathBuf>),
    Io(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::NoLegacy(p) => write!(f, "{} does not exist, so there is nothing to move", p.display()),
            PlanError::Conflicts(c) => {
                writeln!(f, "these already exist in the new folders, so nothing was moved:")?;
                for p in c {
                    writeln!(f, "  {}", p.display())?;
                }
                write!(f, "move or remove them and run --migrate-user-dir again")
            }
            PlanError::Io(e) => write!(f, "{e}"),
        }
    }
}

fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>, PlanError> {
    let rd = std::fs::read_dir(dir).map_err(|e| PlanError::Io(format!("cannot read {}: {e}", dir.display())))?;
    let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    v.sort();
    Ok(v)
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Work out the moves from `legacy` into the roots of `dest`; changes nothing.
pub fn plan(legacy: &Path, dest: &UserDirs) -> Result<Plan, PlanError> {
    if !legacy.is_dir() {
        return Err(PlanError::NoLegacy(legacy.to_path_buf()));
    }
    let mut moves = Vec::new();
    let mut clutter = Vec::new();
    let mut cleanup = Vec::new();
    for path in sorted_children(legacy)? {
        let name = name_of(&path);
        if CLUTTER.contains(&name.as_str()) {
            clutter.push(path);
        } else if name == "cache" && path.is_dir() {
            for child in sorted_children(&path)? {
                let to = dest.cache().join(name_of(&child));
                moves.push(Move { from: child, to });
            }
            cleanup.push(path);
        } else if name == "config.toml" || name == "style.toml" {
            moves.push(Move { to: dest.config().join(&name), from: path });
        } else {
            moves.push(Move { to: dest.data().join(&name), from: path });
        }
    }
    cleanup.push(legacy.to_path_buf());
    let conflicts: Vec<PathBuf> =
        moves.iter().filter(|m| m.to.symlink_metadata().is_ok()).map(|m| m.to.clone()).collect();
    if !conflicts.is_empty() {
        return Err(PlanError::Conflicts(conflicts));
    }
    Ok(Plan { legacy: legacy.to_path_buf(), moves, clutter, cleanup })
}

/// What [`execute`] did, for the report.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub moved: Vec<Move>,
    /// Folders that could not be removed because something is still in them.
    pub left_behind: Vec<PathBuf>,
}

/// A move failed partway: what was done and what was not.
#[derive(Debug)]
pub struct ExecError {
    pub message: String,
    pub moved: Vec<Move>,
    pub not_moved: Vec<Move>,
}

/// Create the parent folders and rename. Nothing is ever copied or deleted here:
/// if a rename is refused (say, the new folder is on another disk), the entry
/// stays where it was and the caller tells the person to move it by hand.
fn move_entry(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
}

/// Carry out `plan`. Stops at the first failed move without rolling back.
pub fn execute(plan: &Plan) -> Result<Outcome, ExecError> {
    let mut out = Outcome::default();
    for (i, m) in plan.moves.iter().enumerate() {
        if let Err(e) = move_entry(&m.from, &m.to) {
            return Err(ExecError {
                message: format!(
                    "could not move {} to {}: {e}; nothing was deleted, so move it by hand",
                    m.from.display(),
                    m.to.display()
                ),
                moved: out.moved,
                not_moved: plan.moves[i..].to_vec(),
            });
        }
        out.moved.push(m.clone());
    }
    for c in &plan.clutter {
        let _ = std::fs::remove_file(c);
    }
    for d in &plan.cleanup {
        if std::fs::remove_dir(d).is_err() {
            out.left_behind.push(d.clone());
        }
    }
    Ok(out)
}

/// The whole interaction over explicit streams; returns the exit code.
pub fn run(
    legacy: &Path,
    dest: &UserDirs,
    yes: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let plan = match plan(legacy, dest) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(err, "lanthorn: {e}");
            return 1;
        }
    };
    let _ = writeln!(out, "Moving {} into the platform folders:", legacy.display());
    for m in &plan.moves {
        let _ = writeln!(out, "  {} -> {}", m.from.display(), m.to.display());
    }
    for c in &plan.clutter {
        let _ = writeln!(out, "  {} (deleted)", c.display());
    }
    if !yes {
        let _ = write!(out, "Move these? [y/N] ");
        let _ = out.flush();
        let mut line = String::new();
        let _ = input.read_line(&mut line);
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            let _ = writeln!(out, "Nothing was changed.");
            return 0;
        }
    }
    match execute(&plan) {
        Ok(o) => {
            for d in &o.left_behind {
                let _ = writeln!(out, "Left in place (not empty): {}", d.display());
            }
            let _ = writeln!(out, "Done. Your files now live in:");
            let _ = writeln!(out, "  config: {}", dest.config().display());
            let _ = writeln!(out, "  data:   {}", dest.data().display());
            let _ = writeln!(out, "  cache:  {}", dest.cache().display());
            0
        }
        Err(e) => {
            let _ = writeln!(err, "lanthorn: {}", e.message);
            let _ = writeln!(err, "Moved before the failure:");
            for m in &e.moved {
                let _ = writeln!(err, "  {} -> {}", m.from.display(), m.to.display());
            }
            let _ = writeln!(err, "Not moved (still in {}):", legacy.display());
            for m in &e.not_moved {
                let _ = writeln!(err, "  {}", m.from.display());
            }
            1
        }
    }
}

/// `--migrate-user-dir` against the real home, stdin and stdout; the exit code.
pub fn run_cli(explicit_user_dir: Option<&Path>, yes: bool) -> i32 {
    if explicit_user_dir.is_some() {
        eprintln!("lanthorn: --migrate-user-dir moves to the platform folders and cannot be combined with --user-dir");
        return 1;
    }
    let Some(home) = home_dir() else {
        eprintln!("lanthorn: cannot find your home directory, so there is no ~/{LEGACY_DIR} to move");
        return 1;
    };
    // The platform defaults: what resolve() says when the legacy folder is not there.
    let inputs = Inputs { legacy_exists: false, explicit: None, ..Inputs::from_environment(None) };
    let dest = match UserDirs::resolve(&inputs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("lanthorn: {e}");
            return 1;
        }
    };
    let stdin = std::io::stdin();
    run(&home.join(LEGACY_DIR), &dest, yes, &mut stdin.lock(), &mut std::io::stdout(), &mut std::io::stderr())
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use crate::scratch_dir;
    use std::fs;

    /// A legacy folder holding one of each kind of entry.
    fn legacy_fixture(home: &Path) -> PathBuf {
        let l = home.join(LEGACY_DIR);
        fs::create_dir_all(l.join("saves/zork")).unwrap();
        fs::write(l.join("saves/zork/a.save"), "s").unwrap();
        fs::create_dir_all(l.join("cache/covers")).unwrap();
        fs::write(l.join("cache/covers/c.png"), "c").unwrap();
        fs::write(l.join("config.toml"), "cfg").unwrap();
        fs::write(l.join("style.toml"), "sty").unwrap();
        fs::write(l.join("crash.log"), "log").unwrap();
        fs::write(l.join("mystery.bin"), "?").unwrap();
        fs::write(l.join(".DS_Store"), "x").unwrap();
        l
    }

    fn split(root: &Path) -> UserDirs {
        UserDirs::new(root.join("cfg"), root.join("data"), root.join("cache"))
    }

    fn go(legacy: &Path, dest: &UserDirs, yes: bool, answer: &str) -> (i32, String, String) {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let code = run(legacy, dest, yes, &mut answer.as_bytes(), &mut o, &mut e);
        (code, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
    }

    fn check_moved(d: &UserDirs, legacy: &Path) {
        assert_eq!(fs::read_to_string(d.config().join("config.toml")).unwrap(), "cfg");
        assert_eq!(fs::read_to_string(d.config().join("style.toml")).unwrap(), "sty");
        assert_eq!(fs::read_to_string(d.cache().join("covers/c.png")).unwrap(), "c");
        assert_eq!(fs::read_to_string(d.data().join("saves/zork/a.save")).unwrap(), "s");
        assert_eq!(fs::read_to_string(d.data().join("crash.log")).unwrap(), "log");
        assert_eq!(fs::read_to_string(d.data().join("mystery.bin")).unwrap(), "?");
        assert!(!d.data().join(".DS_Store").exists() && !d.config().join(".DS_Store").exists());
        assert!(!d.cache().join("cache").exists() && !d.data().join("cache").exists());
        assert!(!legacy.exists(), "the empty legacy folder is removed");
    }

    #[test]
    fn split_roots_get_each_entry_in_its_root() {
        let home = scratch_dir("migrate-split");
        let legacy = legacy_fixture(&home);
        let d = split(&home.join("new"));
        let (code, out, _) = go(&legacy, &d, true, "");
        assert_eq!(code, 0, "{out}");
        check_moved(&d, &legacy);
        assert!(!d.config().join("saves").exists(), "config root holds only config");
        assert!(out.contains("config:") && out.contains("cache:"), "{out}");
    }

    #[test]
    fn coinciding_roots_work() {
        let home = scratch_dir("migrate-coincide");
        let legacy = legacy_fixture(&home);
        let shared = home.join("new/support");
        let d = UserDirs::new(&shared, &shared, home.join("new/caches"));
        let (code, out, _) = go(&legacy, &d, true, "");
        assert_eq!(code, 0, "{out}");
        check_moved(&d, &legacy);
    }

    #[test]
    fn conflicts_refuse_and_change_nothing() {
        let home = scratch_dir("migrate-conflict");
        let legacy = legacy_fixture(&home);
        let d = split(&home.join("new"));
        fs::create_dir_all(d.data().join("saves")).unwrap();
        fs::create_dir_all(d.config()).unwrap();
        fs::write(d.config().join("config.toml"), "mine").unwrap();
        let (code, _, err) = go(&legacy, &d, true, "");
        assert_ne!(code, 0);
        assert!(err.contains("saves") && err.contains("config.toml"), "{err}");
        assert!(legacy.join("saves/zork/a.save").exists() && legacy.join(".DS_Store").exists());
        assert_eq!(fs::read_to_string(d.config().join("config.toml")).unwrap(), "mine");
        assert!(!d.cache().exists() && !d.data().join("crash.log").exists());
    }

    #[test]
    fn missing_legacy_is_refused() {
        let home = scratch_dir("migrate-missing");
        let (code, _, err) = go(&home.join(LEGACY_DIR), &split(&home.join("new")), true, "");
        assert_ne!(code, 0);
        assert!(err.contains("nothing to move"), "{err}");
    }

    #[test]
    fn answering_no_changes_nothing_and_succeeds() {
        let home = scratch_dir("migrate-no");
        let legacy = legacy_fixture(&home);
        let d = split(&home.join("new"));
        for answer in ["", "n\n", "\n"] {
            let (code, out, _) = go(&legacy, &d, false, answer);
            assert_eq!(code, 0);
            assert!(out.contains("Move these? [y/N]") && out.contains("Nothing was changed"), "{out}");
            assert!(legacy.join("config.toml").exists() && !d.config().exists());
        }
        let (code, _, _) = go(&legacy, &d, false, "y\n");
        assert_eq!(code, 0);
        check_moved(&d, &legacy);
    }

    #[test]
    fn leftovers_keep_the_legacy_folder() {
        let home = scratch_dir("migrate-left");
        let legacy = legacy_fixture(&home);
        let d = split(&home.join("new"));
        let p = plan(&legacy, &d).unwrap();
        // Something appears after planning; remove_dir must not take it with it.
        fs::write(legacy.join("late.txt"), "x").unwrap();
        let o = execute(&p).unwrap();
        assert_eq!(o.left_behind, vec![legacy.clone()]);
        assert!(legacy.join("late.txt").exists());
    }
}
