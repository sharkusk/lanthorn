//! Per-story disk sidecar for the Glulx Glk file VFS (SQ-0278).
//! Path + filesystem only: the bytes are already the gvm sidecar blob
//! (`session.vfs_bytes()`, encoded by `gvm::glk::encode_files`), so this
//! module does not touch the wire format. Mirrors `aux_store` for the
//! Z-machine aux table.

use std::path::{Path, PathBuf};

/// `<game_dir>/default.glkvfs` (SQ-0284). The VFS sidecar is the game's
/// singleton Glk file store, kept under the per-game directory keyed by story
/// filename.
pub fn vfs_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.glkvfs")
}

/// Read the per-game VFS sidecar (empty bytes if absent or unreadable).
pub fn read_vfs(game_dir: &Path) -> Vec<u8> {
    std::fs::read(vfs_path(game_dir)).unwrap_or_default()
}

/// Write the per-game VFS sidecar (creating `game_dir` if needed).
///
/// Atomic (SQ-0644): this is the game's whole Glk file store in one blob, so a
/// half-written sidecar would cost every file the story has ever created.
pub fn write_vfs(game_dir: &Path, bytes: &[u8]) -> std::io::Result<()> {
    crate::storage::atomic_write(&vfs_path(game_dir), bytes)
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    #[test]
    fn vfs_path_is_default_glkvfs_in_game_dir() {
        let dir = Path::new("/tmp/saves/Advent.gblorb");
        let p = vfs_path(dir);
        assert_eq!(p, PathBuf::from("/tmp/saves/Advent.gblorb/default.glkvfs"));
        assert_eq!(p.parent(), Some(dir), "stays in the game dir");
    }

    #[test]
    fn round_trips_through_temp_dir() {
        let dir = std::env::temp_dir().join(format!("lanthorn-vfs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_vfs(&dir).is_empty(), "absent file → empty");
        let blob = vec![0xDE, 0xAD, 0xBE, 0xEF];
        write_vfs(&dir, &blob).unwrap();
        assert_eq!(read_vfs(&dir), blob);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0644: the sidecar is the game's WHOLE Glk file store in one blob, so a
    /// truncated write costs every file the story ever created. The write goes
    /// temp-then-rename, which a directory that admits no new files proves: it fails
    /// outright instead of half-happening (an in-place `fs::write` would have
    /// succeeded and clobbered the blob).
    #[test]
    fn an_interrupted_vfs_write_keeps_the_previous_blob() {
        let dir = std::env::temp_dir().join(format!("lanthorn-vfs-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blob = vec![0xDE, 0xAD, 0xBE, 0xEF];
        write_vfs(&dir, &blob).unwrap();

        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let result = write_vfs(&dir, b"replacement");
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a write that cannot complete must fail, not half-happen");
        assert_eq!(read_vfs(&dir), blob, "the previous sidecar is still readable");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
