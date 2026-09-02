//! Per-game storage layout (SQ-0284). Saves and sidecars live in a flat
//! per-game directory `<base>/<story-key>.save/`. The `.save` suffix keeps the
//! directory from colliding with the story file when `base` is the story's own
//! directory. Inside it: `default.aux`, `default.glkvfs`,
//! `default.lanthorn` (auto/singleton), plus `<slug>.lanthorn` / `<slug>.qzl`
//! (named), and the per-game `style.toml` / `config.toml` overrides (SQ-0346).
//!
//! **The key is the story's filename, unless the story came off a disk image, in
//! which case it is that story's own release and serial** (SQ-0850). One image
//! held one game until the corpus grew compilations — `Infocom Compilation 1
//! (19xx)(-).st` alone carries six — and a filename key gave all six of them one
//! `default.lanthorn` to overwrite in turn. The rule and its reasoning live in
//! [`cli_host::storage`], which `zvm-cli` reads from too, so the TUI and the CLI
//! cannot name the same game's directory two ways. IFID is retained elsewhere
//! for title/hint lookup only.
//!
//! A story taken out of a **zip** has no build, so it is keyed by its entry's
//! basename — the same token the same game keys on loose (SQ-1098). All three
//! rules read one [`StoryOrigin`], so a caller cannot supply a subset and get a
//! plausible answer.

pub use cli_host::storage::{
    DiskBuild, StoryOrigin, story_key_at, story_key_at_from, story_key_for,
};

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The per-game directory: `<base>/<story-key>.save/`. The `.save` suffix
/// keeps the directory from colliding with the story file itself when `base`
/// is the story's own directory (e.g. `Zork1.z5` vs `Zork1.z5.save/`).
pub fn game_dir(base: &Path, key: &str) -> PathBuf {
    base.join(format!("{key}.save"))
}

/// The default (auto/singleton) Save-State slot inside a game dir.
pub fn default_state_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.lanthorn")
}

/// `default` is reserved for the auto/singleton slot; a user save may not use it.
pub fn is_reserved_slug(slug: &str) -> bool {
    slug == "default"
}

/// Delete the game's AUTO persistent data so the next boot starts from scratch:
/// the Glk VFS cache (`default.glkvfs`), the Z-machine aux sidecar (`default.aux`),
/// the auto/singleton Save State (`default.lanthorn`), and the debug executed-PC
/// coverage sidecar (`default.pcs`). The player's named Save States
/// (`<slug>.lanthorn`) and in-game saves (`<slug>.qzl` / `_*.qzl`) are left
/// untouched — only the reserved `default.*` files go. A missing file is not an
/// error.
pub fn delete_auto_persistent(game_dir: &Path) {
    for name in ["default.glkvfs", "default.aux", "default.lanthorn", "default.pcs"] {
        let path = game_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("warning: could not delete {}: {e}", path.display()),
        }
    }
}

// ── atomic writes (SQ-0644) ───────────────────────────────────────────────────

/// Serial for temp-file names, so two writes aimed at the same target from one
/// process (two threads, or a retry after a failure) can never share a temp path.
static TMP_SERIAL: AtomicU64 = AtomicU64::new(0);

/// The temp sibling a write to `path` builds in: `.<name>.part-<pid>-<serial>`,
/// always in the SAME directory (a rename is only atomic within one filesystem).
///
/// The suffix goes AFTER the real extension on purpose: `list_saves` and friends
/// match on `.lanthorn` / `.qzl` endings, so an abandoned temp is invisible to
/// every picker and scan rather than showing up as a half-written save.
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("lanthorn");
    let serial = TMP_SERIAL.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.part-{}-{serial}", std::process::id()))
}

/// Write `bytes` to `path` atomically (SQ-0644).
///
/// Every persistent file lanthorn owns goes through here. `File::create` on the
/// final path truncates it *before* the replacement is durable, and these writers
/// run at the worst possible moments — the auto-save rewrites `default.lanthorn`
/// on EVERY turn, and the quit path can be cut short by the exit watchdog — so a
/// crash, a power loss, or a killed process left a zero-length or half-written
/// file where the player's game used to be.
///
/// Instead: build the new contents in a temp file beside the target, fsync it,
/// then `rename` over the target. Rename is atomic within a filesystem, so a
/// reader (this run or the next) sees either the old file or the new one, never a
/// splice of the two. The fsync is what makes that true across a power loss and
/// not merely across a crash; these files are small and written at most once a
/// turn, so the cost is paid gladly.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(path, |tmp| {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })
}

/// [`atomic_write`] for content a caller must stream (the `.lanthorn` ZIP): `build`
/// is handed the temp path and writes whatever it likes there. The file is renamed
/// over `path` only if `build` returns `Ok`; on any error the temp is removed and
/// the target is left exactly as it was.
pub fn atomic_write_with<F>(path: &Path, build: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_sibling(path);
    if let Err(e) = build(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Test support: make `dir` reject NEW files, which is exactly what separates an
/// atomic write from an in-place one — a temp-then-rename write cannot even start,
/// while `File::create`/`fs::write` on an existing, still-writable file happily
/// truncates it. Returns false when the platform (or running as root) can't
/// enforce it, so the caller skips vacuously.
#[cfg(all(test, unix, feature = "t-persist"))]
pub(crate) fn deny_new_files_in(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Ok(md) = std::fs::metadata(dir) else { return false };
    let mut perm = md.permissions();
    perm.set_mode(0o500); // r-x------: existing files stay writable, new ones can't be made
    if std::fs::set_permissions(dir, perm).is_err() {
        return false;
    }
    // root ignores the mode bits — probe, and give up on the test if it got through.
    let probe = dir.join(".deny-probe");
    if std::fs::File::create(&probe).is_ok() {
        let _ = std::fs::remove_file(&probe);
        allow_new_files_in(dir);
        return false;
    }
    true
}

#[cfg(all(test, not(unix), feature = "t-persist"))]
pub(crate) fn deny_new_files_in(_dir: &Path) -> bool {
    false
}

/// Undo [`deny_new_files_in`] so the test can clean up after itself.
#[cfg(all(test, unix, feature = "t-persist"))]
pub(crate) fn allow_new_files_in(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(md) = std::fs::metadata(dir) {
        let mut perm = md.permissions();
        perm.set_mode(0o700);
        let _ = std::fs::set_permissions(dir, perm);
    }
}

#[cfg(all(test, not(unix), feature = "t-persist"))]
pub(crate) fn allow_new_files_in(_dir: &Path) {}

/// Test support: the temp files [`atomic_write`] would have left behind.
#[cfg(all(test, feature = "t-persist"))]
pub(crate) fn leftover_temps(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|n| n.contains(".part-"))
        .collect()
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A loose story file — anything that is not a disk image — keys on its
    /// basename, exactly as it did before SQ-0850. Nobody's saves move.
    #[test]
    fn key_keeps_ext_and_sanitizes() {
        let loose = |p: &'static str| {
            story_key_for(StoryOrigin { path: Path::new(p), entry: None, build: None })
        };
        assert_eq!(loose("/g/Zork1.z5"), "Zork1.z5");
        assert_ne!(loose("/g/z.z5"), loose("/g/z.gblorb"));
        assert_eq!(loose("/g/a b?.z5"), "a_b_.z5");
        assert_eq!(loose(""), "game");
    }

    #[test]
    fn game_dir_joins() {
        assert_eq!(game_dir(Path::new("/base"), "Zork1.z5"), PathBuf::from("/base/Zork1.z5.save"));
    }

    /// The per-game dir name must always end in `.save` — that's what keeps
    /// it from colliding with a story file of the same name.
    #[test]
    fn game_dir_name_ends_with_dot_save() {
        let dir = game_dir(Path::new("/base"), "Advent.gblorb");
        assert!(
            dir.file_name().unwrap().to_str().unwrap().ends_with(".save"),
            "got {dir:?}"
        );
    }

    /// Regression for SQ-0284: when `base` is the story's own directory (the
    /// CLI default) and a file already exists at `<base>/<story-key>`,
    /// `create_dir_all(game_dir(..))` must still succeed because the `.save`
    /// suffix makes the two paths distinct.
    #[test]
    fn game_dir_does_not_collide_with_same_named_story_file() {
        let tmp = std::env::temp_dir().join(format!("lanthorn-storage-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.gblorb");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.gblorb

        let dir = game_dir(&tmp, &story_key_at(&story_path));
        assert_eq!(dir, tmp.join("game.gblorb.save"));
        std::fs::create_dir_all(&dir).expect("must not collide with the story file");
        assert!(dir.is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_state_path_is_in_game_dir() {
        assert_eq!(
            default_state_path(Path::new("/base/Zork1.z5.save")),
            PathBuf::from("/base/Zork1.z5.save/default.lanthorn")
        );
    }

    #[test]
    fn default_is_reserved() {
        assert!(is_reserved_slug("default"));
        assert!(!is_reserved_slug("quicksave"));
    }

    /// `delete_auto_persistent` removes exactly the three reserved `default.*`
    /// auto files and keeps the player's named Save States and in-game saves.
    #[test]
    fn delete_auto_persistent_removes_only_defaults() {
        let tmp = std::env::temp_dir()
            .join(format!("lanthorn-delete-auto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // The auto sidecars that must go.
        for f in ["default.glkvfs", "default.aux", "default.lanthorn", "default.pcs"] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }
        // Player data that must survive.
        for f in ["myslot.lanthorn", "quick.qzl", "_autosave.qzl"] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        delete_auto_persistent(&tmp);

        for f in ["default.glkvfs", "default.aux", "default.lanthorn", "default.pcs"] {
            assert!(!tmp.join(f).exists(), "{f} should be deleted");
        }
        for f in ["myslot.lanthorn", "quick.qzl", "_autosave.qzl"] {
            assert!(tmp.join(f).exists(), "{f} should be kept");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── atomic writes (SQ-0644) ──────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        crate::scratch_dir(tag)
    }

    /// The happy path: the target ends up with the new bytes and the temp file is
    /// gone — nothing for a picker or a scan to trip over.
    #[test]
    fn atomic_write_replaces_the_target_and_cleans_up() {
        let dir = scratch("atomic-ok");
        let target = dir.join("default.lanthorn");
        std::fs::write(&target, b"old").unwrap();
        atomic_write(&target, b"new contents").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new contents");
        assert!(leftover_temps(&dir).is_empty(), "temp file must not survive a good write");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole point: a write that dies partway leaves the PREVIOUS file intact.
    /// The half-written bytes only ever exist in the temp sibling, which is removed.
    #[test]
    fn an_interrupted_write_leaves_the_old_file_readable() {
        let dir = scratch("atomic-fail");
        let target = dir.join("default.lanthorn");
        std::fs::write(&target, b"the previous good save").unwrap();

        let err = atomic_write_with(&target, |tmp| {
            std::fs::write(tmp, b"half a sav")?; // …and then the process dies
            Err(io::Error::other("interrupted"))
        })
        .expect_err("the failure propagates");
        assert_eq!(err.to_string(), "interrupted");

        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"the previous good save",
            "the old file is untouched — never truncated, never spliced",
        );
        assert!(leftover_temps(&dir).is_empty(), "the partial temp is cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The target directory is created on demand, as the old `fs::write` callers
    /// did for themselves.
    #[test]
    fn atomic_write_creates_the_parent_directory() {
        let dir = scratch("atomic-mkdir");
        let target = dir.join("nested").join("deeper").join("default.aux");
        atomic_write(&target, b"x").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing file (or a fresh game dir) is not an error.
    #[test]
    fn delete_auto_persistent_ignores_missing() {
        let tmp = std::env::temp_dir()
            .join(format!("lanthorn-delete-auto-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        delete_auto_persistent(&tmp); // no default.* present -> no panic
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
