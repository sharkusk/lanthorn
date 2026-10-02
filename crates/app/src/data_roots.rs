//! Who is playing, and where their files live (SQ-1676).
//!
//! One install can serve several players. What every player shares is the
//! **catalogue**: per-story `info.json`, `cover.png`, the IFDB link state and the
//! Glulx learned-address files, all under `<user_dir>/saves/<story-key>.save/`.
//! What is per-player is everything else a story folder holds (saves, aux data,
//! exports, per-game sidecars) and the layered `config.toml` / `style.toml`.
//!
//! Lanthorn never authenticates. A player is just a name, handed in by
//! `--player` / `LANTHORN_PLAYER`; whatever sits in front of lanthorn decides
//! who is allowed to claim it.
//!
//! The two bases travel together as one [`DataRoots`], like `MachineBoot` does
//! for a machine's boot facts: a caller that holds only "a data base" cannot
//! tell whether it is about to write a save or a cover.

use std::path::{Path, PathBuf};

/// The longest player name accepted (ttyd's `TTYD_USER` caps a user at 29).
pub const MAX_PLAYER_NAME: usize = 29;

/// The environment variable that names the player when `--player` is absent.
pub const PLAYER_ENV: &str = "LANTHORN_PLAYER";

/// The two storage bases a launch works against.
///
/// * `catalogue` holds what every player shares — see the module docs.
/// * `player` holds this player's own per-story state.
///
/// For the default player (and for every single-player install, which is every
/// install that never names a player) the two are the same directory, which is
/// today's layout exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoots {
    catalogue: PathBuf,
    player: PathBuf,
}

impl DataRoots {
    /// One directory serving both halves: the default player's layout.
    pub fn single(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        DataRoots { catalogue: base.clone(), player: base }
    }

    /// The roots for a launch.
    ///
    /// The rule for the three flags: `--data-dir` stands in for
    /// `<user-dir>/saves`, the shared catalogue base (and the default player's
    /// saves, as it always has been); `--player` always roots under
    /// `<user-dir>/users/<name>/`, wherever the catalogue was moved to.
    pub fn resolve(user_dir: &Path, data_dir: Option<&Path>, player: Option<&str>) -> Self {
        let catalogue = data_dir.map(Path::to_path_buf).unwrap_or_else(|| user_dir.join("saves"));
        let player = match player {
            Some(name) => player_root(user_dir, name).join("saves"),
            None => catalogue.clone(),
        };
        DataRoots { catalogue, player }
    }

    /// The shared catalogue base.
    pub fn catalogue(&self) -> &Path {
        &self.catalogue
    }

    /// This player's base.
    pub fn player(&self) -> &Path {
        &self.player
    }

    /// Where this story's shared metadata lives.
    pub fn catalogue_dir(&self, key: &str) -> PathBuf {
        crate::storage::game_dir(&self.catalogue, key)
    }

    /// Where this player's saves and sidecars for the story live.
    pub fn player_dir(&self, key: &str) -> PathBuf {
        crate::storage::game_dir(&self.player, key)
    }
}

/// `<user_dir>/users/<name>`: one non-default player's root.
pub fn player_root(user_dir: &Path, name: &str) -> PathBuf {
    user_dir.join("users").join(name)
}

/// Check a player name: `[A-Za-z0-9._-]`, 1 to 29 characters, no leading `.`
/// (which also rules out `.` and `..`).
pub fn validate_player_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_PLAYER_NAME {
        return Err(format!("a player name must be 1 to {MAX_PLAYER_NAME} characters, got {}", name.len()));
    }
    if name.starts_with('.') {
        return Err("a player name must not start with '.'".to_string());
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))) {
        return Err(format!("a player name may use only letters, digits, '.', '_' and '-' (found {c:?})"));
    }
    Ok(())
}

/// Pick the player for a launch: the flag wins over the environment; absent or
/// empty on either means the default player (`None`). An invalid name is an error.
pub fn select_player(flag: Option<&str>, env: Option<&str>) -> Result<Option<String>, String> {
    let chosen = flag.or(env).unwrap_or("");
    if chosen.is_empty() {
        return Ok(None);
    }
    validate_player_name(chosen).map_err(|e| format!("invalid player name {chosen:?}: {e}"))?;
    Ok(Some(chosen.to_string()))
}

/// [`select_player`] reading the real environment.
pub fn select_player_from_env(flag: Option<&str>) -> Result<Option<String>, String> {
    select_player(flag, std::env::var(PLAYER_ENV).ok().as_deref())
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        for ok in ["bob", "Bob_2", "a.b-c", "x", &"a".repeat(29)] {
            assert!(validate_player_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "a b", "bob\\x", "é", &"a".repeat(30), "a\0b"] {
            assert!(validate_player_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn flag_beats_env_and_empty_is_default() {
        assert_eq!(select_player(Some("amy"), Some("bob")).unwrap().as_deref(), Some("amy"));
        assert_eq!(select_player(None, Some("bob")).unwrap().as_deref(), Some("bob"));
        assert_eq!(select_player(None, None).unwrap(), None);
        assert_eq!(select_player(None, Some("")).unwrap(), None);
        // An explicit empty flag is the default player even with the env set.
        assert_eq!(select_player(Some(""), Some("bob")).unwrap(), None);
        assert!(select_player(None, Some("../x")).is_err());
        assert!(select_player(Some(".."), None).is_err());
    }

    #[test]
    fn default_player_layout_is_unchanged() {
        let u = Path::new("/u");
        let r = DataRoots::resolve(u, None, None);
        assert_eq!(r.catalogue(), Path::new("/u/saves"));
        assert_eq!(r.player(), Path::new("/u/saves"));
        assert_eq!(r.player_dir("z.z5"), Path::new("/u/saves/z.z5.save"));
    }

    #[test]
    fn named_player_roots_under_users_and_data_dir_moves_only_the_catalogue() {
        let u = Path::new("/u");
        let r = DataRoots::resolve(u, None, Some("bob"));
        assert_eq!(r.catalogue(), Path::new("/u/saves"));
        assert_eq!(r.player(), Path::new("/u/users/bob/saves"));
        let r = DataRoots::resolve(u, Some(Path::new("/d")), Some("bob"));
        assert_eq!(r.catalogue(), Path::new("/d"));
        assert_eq!(r.player(), Path::new("/u/users/bob/saves"));
        let r = DataRoots::resolve(u, Some(Path::new("/d")), None);
        assert_eq!((r.catalogue(), r.player()), (Path::new("/d"), Path::new("/d")));
    }
}
