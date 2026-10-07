//! Where lanthorn keeps a user's files: the four roots (SQ-1721, SQ-1722, SQ-1726).
//!
//! Everything lanthorn writes for a person hangs off four roots, carried together
//! as one [`UserDirs`] the way `DataRoots` and `MachineBoot` carry their facts:
//!
//! | root | holds |
//! |---|---|
//! | `config` | `config.toml`, `style.toml` |
//! | `data` | `saves/`, `users/<name>/`, `documents/`, the user's system disks |
//! | `cache` | only what is regenerated with no network and no user action |
//! | `logs` | `crash.log`, `trace.log`, `stderr.log`, the `/dump-*` logs |
//!
//! The rules (`docs/internals/user-dirs.md`), highest first:
//!
//! 1. an **explicit** value: a `UserDirs` an embedding host hands in, or
//!    `--user-dir X` (the single-folder layout inside X);
//! 2. **legacy**: `~/.lanthorn` already exists, so it is used whole, as before;
//! 3. the **platform defaults** (macOS and Linux XDG,
//!    Windows `%APPDATA%` / `%LOCALAPPDATA%`), under the plain name `lanthorn`.
//!
//! [`UserDirs::resolve`] is a pure function over an [`Inputs`] value, so every
//! platform's row is testable on any OS; [`Inputs::from_environment`] is the only
//! place the real environment is read. There is exactly one home-directory helper,
//! [`home_dir`] (on `dirs::home_dir()`, which on Windows is the profile from the
//! Known Folder API, so an unset `HOME` does not matter there). Nothing falls back
//! to `.`: when the folders cannot be worked out at all, [`UserDirs::detect`]
//! returns a [`NoHome`] error, which startup reports and exits on.

use std::fmt;
use std::path::{Path, PathBuf};

/// The folder name under each platform's base directory.
pub const APP_DIR: &str = "lanthorn";

/// The legacy single-folder home, `~/.lanthorn`.
pub const LEGACY_DIR: &str = ".lanthorn";

/// The four roots; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDirs {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

/// The folders could not be worked out: no home directory (and, on Windows, no
/// profile folders either).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoHome;

impl fmt::Display for NoHome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot find your home directory, so cannot choose where to keep lanthorn's files; \
             pass --user-dir <folder> to say where"
        )
    }
}

impl std::error::Error for NoHome {}

/// The platform whose defaults [`UserDirs::resolve`] applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Takes the same XDG layout as Linux: terminal tools on macOS keep their
    /// files under `~/.config`, not `~/Library`.
    MacOs,
    /// Linux and every other Unix: the XDG layout.
    Linux,
    Windows,
}

impl Platform {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Every fact [`UserDirs::resolve`] reads, so the rule is a pure function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    pub platform: Platform,
    /// The home directory ([`home_dir`]); `None` when it cannot be found.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME` as set. Read on
    /// Linux and macOS; an empty or relative value is ignored, per the XDG specification.
    pub xdg_config: Option<PathBuf>,
    pub xdg_data: Option<PathBuf>,
    pub xdg_cache: Option<PathBuf>,
    pub xdg_state: Option<PathBuf>,
    /// Windows' roaming and local app-data folders (`%APPDATA%`,
    /// `%LOCALAPPDATA%`). Only read on Windows.
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    /// Whether `<home>/.lanthorn` exists.
    pub legacy_exists: bool,
    /// `--user-dir`.
    pub explicit: Option<PathBuf>,
}

impl Inputs {
    /// Inputs with nothing set, for `platform`: the starting point for tests and
    /// for hosts that fill in only what they know.
    pub fn empty(platform: Platform) -> Self {
        Inputs {
            platform,
            home: None,
            xdg_config: None,
            xdg_data: None,
            xdg_cache: None,
            xdg_state: None,
            appdata: None,
            local_appdata: None,
            legacy_exists: false,
            explicit: None,
        }
    }

    /// Read the real environment. The ONLY function here that does.
    pub fn from_environment(explicit: Option<&Path>) -> Self {
        let platform = Platform::current();
        let home = home_dir();
        let env_path = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let (appdata, local_appdata) = if platform == Platform::Windows {
            // The Known Folder API, not the environment: it answers even when a
            // launcher scrubbed the variables.
            (dirs::config_dir(), dirs::cache_dir())
        } else {
            (None, None)
        };
        Inputs {
            platform,
            legacy_exists: home.as_ref().is_some_and(|h| h.join(LEGACY_DIR).exists()),
            home,
            xdg_config: env_path("XDG_CONFIG_HOME"),
            xdg_data: env_path("XDG_DATA_HOME"),
            xdg_cache: env_path("XDG_CACHE_HOME"),
            xdg_state: env_path("XDG_STATE_HOME"),
            appdata,
            local_appdata,
            explicit: explicit.map(Path::to_path_buf),
        }
    }
}

/// The one home-directory helper. `None` when it cannot be found; callers decide
/// what that means and none of them may substitute the working directory.
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir().filter(|p| !p.as_os_str().is_empty())
}

/// An XDG base directory value: honoured only when non-empty and absolute.
fn xdg(value: &Option<PathBuf>) -> Option<&Path> {
    // Judged by the Unix rule (starts with `/`), not the host's `Path::is_absolute`:
    // this is the Linux row, and on a Windows host `/x/cfg` has no drive letter.
    value.as_deref().filter(|p| p.to_string_lossy().starts_with('/'))
}

impl UserDirs {
    /// Hosts that supply all four roots themselves.
    pub fn new(
        config: impl Into<PathBuf>,
        data: impl Into<PathBuf>,
        cache: impl Into<PathBuf>,
        logs: impl Into<PathBuf>,
    ) -> Self {
        UserDirs { config: config.into(), data: data.into(), cache: cache.into(), logs: logs.into() }
    }

    /// The single-folder layout: config, `saves/`, `documents/` and `cache/` all
    /// inside `base`, and the log files beside the data. What `--user-dir` and the legacy `~/.lanthorn` mean.
    pub fn single(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        UserDirs { config: base.clone(), cache: base.join("cache"), logs: base.clone(), data: base }
    }

    /// Where `config.toml` and `style.toml` live.
    pub fn config(&self) -> &Path {
        &self.config
    }

    /// Where `saves/`, `users/`, `documents/` and the user's system disks live.
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// Where regenerable-without-network files live.
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// Where `crash.log`, `trace.log`, `stderr.log` and the `/dump-*` logs go.
    pub fn logs(&self) -> &Path {
        &self.logs
    }

    /// Choose the roots: explicit, then legacy, then the platform defaults.
    pub fn resolve(inputs: &Inputs) -> Result<Self, NoHome> {
        if let Some(dir) = &inputs.explicit {
            return Ok(Self::single(dir));
        }
        if let (Some(home), true) = (&inputs.home, inputs.legacy_exists) {
            return Ok(Self::single(home.join(LEGACY_DIR)));
        }
        let home = inputs.home.as_deref();
        match inputs.platform {
            Platform::Linux | Platform::MacOs => {
                let pick = |x: &Option<PathBuf>, under_home: &[&str]| -> Result<PathBuf, NoHome> {
                    match xdg(x) {
                        Some(base) => Ok(base.join(APP_DIR)),
                        None => {
                            let base = under_home.iter().fold(home.ok_or(NoHome)?.to_path_buf(), |p, s| p.join(s));
                            Ok(base.join(APP_DIR))
                        }
                    }
                };
                Ok(UserDirs {
                    config: pick(&inputs.xdg_config, &[".config"])?,
                    data: pick(&inputs.xdg_data, &[".local", "share"])?,
                    cache: pick(&inputs.xdg_cache, &[".cache"])?,
                    logs: pick(&inputs.xdg_state, &[".local", "state"])?,
                })
            }
            Platform::Windows => {
                // The profile's folders; with no app-data folder given, derive it
                // from the profile (the standard AppData\Roaming / AppData\Local).
                let roaming = match &inputs.appdata {
                    Some(p) => p.clone(),
                    None => home.ok_or(NoHome)?.join("AppData").join("Roaming"),
                };
                let local = match &inputs.local_appdata {
                    Some(p) => p.clone(),
                    None => home.ok_or(NoHome)?.join("AppData").join("Local"),
                };
                let base = roaming.join(APP_DIR);
                let local = local.join(APP_DIR);
                Ok(UserDirs { config: base.clone(), data: base, logs: local.join("logs"), cache: local })
            }
        }
    }

    /// [`UserDirs::resolve`] against the real environment, with `--user-dir`.
    pub fn detect(explicit: Option<&Path>) -> Result<Self, NoHome> {
        Self::resolve(&Inputs::from_environment(explicit))
    }

    /// [`UserDirs::detect`] for the places that cannot return an error (a bare
    /// `Config::default()`): when there is no home at all, a folder under the OS
    /// temp directory, never the working directory. Startup runs
    /// [`UserDirs::detect`] first and exits on the error, so a real launch never
    /// reaches this fallback.
    pub fn detect_or_temp(explicit: Option<&Path>) -> Self {
        Self::detect(explicit).unwrap_or_else(|_| Self::single(std::env::temp_dir().join("lanthorn-no-home")))
    }
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn with_home(platform: Platform, home: &str) -> Inputs {
        Inputs { home: Some(p(home)), ..Inputs::empty(platform) }
    }

    fn roots(d: &UserDirs) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        (d.config().to_path_buf(), d.data().to_path_buf(), d.cache().to_path_buf(), d.logs().to_path_buf())
    }

    #[test]
    fn macos_uses_the_xdg_dot_dirs_not_library() {
        let d = UserDirs::resolve(&with_home(Platform::MacOs, "/Users/ann")).unwrap();
        assert_eq!(
            roots(&d),
            (
                p("/Users/ann/.config/lanthorn"),
                p("/Users/ann/.local/share/lanthorn"),
                p("/Users/ann/.cache/lanthorn"),
                p("/Users/ann/.local/state/lanthorn"),
            )
        );
    }

    #[test]
    fn linux_without_xdg_uses_the_dot_dirs() {
        let d = UserDirs::resolve(&with_home(Platform::Linux, "/home/ann")).unwrap();
        assert_eq!(
            roots(&d),
            (
                p("/home/ann/.config/lanthorn"),
                p("/home/ann/.local/share/lanthorn"),
                p("/home/ann/.cache/lanthorn"),
                p("/home/ann/.local/state/lanthorn"),
            )
        );
    }

    #[test]
    fn linux_honours_absolute_xdg_vars() {
        let i = Inputs {
            xdg_config: Some(p("/x/cfg")),
            xdg_data: Some(p("/x/data")),
            xdg_cache: Some(p("/x/cache")),
            xdg_state: Some(p("/x/state")),
            ..with_home(Platform::Linux, "/home/ann")
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(
            roots(&d),
            (p("/x/cfg/lanthorn"), p("/x/data/lanthorn"), p("/x/cache/lanthorn"), p("/x/state/lanthorn"))
        );
    }

    #[test]
    fn linux_ignores_relative_and_empty_xdg_values() {
        let i = Inputs {
            xdg_config: Some(p("relative/cfg")),
            xdg_data: Some(p("")),
            xdg_cache: Some(p("../c")),
            xdg_state: Some(p("state")),
            ..with_home(Platform::Linux, "/home/ann")
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(
            roots(&d),
            (
                p("/home/ann/.config/lanthorn"),
                p("/home/ann/.local/share/lanthorn"),
                p("/home/ann/.cache/lanthorn"),
                p("/home/ann/.local/state/lanthorn"),
            )
        );
    }

    #[test]
    fn linux_xdg_without_home_still_resolves() {
        let i = Inputs {
            xdg_config: Some(p("/x/cfg")),
            xdg_data: Some(p("/x/data")),
            xdg_cache: Some(p("/x/cache")),
            xdg_state: Some(p("/x/state")),
            ..Inputs::empty(Platform::Linux)
        };
        assert!(UserDirs::resolve(&i).is_ok());
    }

    #[test]
    fn macos_honours_absolute_xdg_vars() {
        let i = Inputs {
            xdg_config: Some(p("/x/cfg")),
            xdg_state: Some(p("/x/state")),
            xdg_data: Some(p("rel/data")),
            ..with_home(Platform::MacOs, "/Users/ann")
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(d.config(), p("/x/cfg/lanthorn"));
        assert_eq!(d.logs(), p("/x/state/lanthorn"));
        assert_eq!(d.data(), p("/Users/ann/.local/share/lanthorn"), "a relative value is ignored");
    }

    #[test]
    fn windows_uses_roaming_for_config_and_data_and_local_for_cache() {
        let i = Inputs {
            appdata: Some(p("C:\\Users\\ann\\AppData\\Roaming")),
            local_appdata: Some(p("C:\\Users\\ann\\AppData\\Local")),
            ..with_home(Platform::Windows, "C:\\Users\\ann")
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(d.config(), p("C:\\Users\\ann\\AppData\\Roaming").join("lanthorn"));
        assert_eq!(d.data(), d.config());
        assert_eq!(d.cache(), p("C:\\Users\\ann\\AppData\\Local").join("lanthorn"));
        assert_eq!(d.logs(), p("C:\\Users\\ann\\AppData\\Local").join("lanthorn").join("logs"));
    }

    /// SQ-1721: a Windows process with no `HOME` (the usual case) still resolves,
    /// from the profile folders alone.
    #[test]
    fn windows_with_no_home_resolves_from_the_profile_folders() {
        let i = Inputs {
            appdata: Some(p("C:\\Users\\ann\\AppData\\Roaming")),
            local_appdata: Some(p("C:\\Users\\ann\\AppData\\Local")),
            ..Inputs::empty(Platform::Windows)
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(d.data(), p("C:\\Users\\ann\\AppData\\Roaming").join("lanthorn"));
    }

    #[test]
    fn windows_derives_app_data_from_the_profile_when_not_given() {
        let d = UserDirs::resolve(&with_home(Platform::Windows, "C:\\Users\\ann")).unwrap();
        assert_eq!(d.config(), p("C:\\Users\\ann").join("AppData").join("Roaming").join("lanthorn"));
        assert_eq!(d.cache(), p("C:\\Users\\ann").join("AppData").join("Local").join("lanthorn"));
        assert_eq!(d.logs(), d.cache().join("logs"));
    }

    #[test]
    fn no_home_and_nothing_else_is_an_error_never_the_working_directory() {
        for platform in [Platform::MacOs, Platform::Linux, Platform::Windows] {
            assert_eq!(UserDirs::resolve(&Inputs::empty(platform)), Err(NoHome), "{platform:?}");
        }
    }

    #[test]
    fn the_fallback_for_infallible_callers_is_never_relative() {
        // `detect_or_temp` is what a bare Config::default() uses; whatever the
        // environment, it must not be the working directory.
        let d = UserDirs::detect_or_temp(None);
        assert!(d.data().is_absolute(), "{:?}", d.data());
        assert!(d.config().is_absolute() && d.cache().is_absolute() && d.logs().is_absolute());
    }

    #[test]
    fn a_legacy_home_is_used_whole_on_every_platform() {
        for (platform, home) in [
            (Platform::MacOs, "/Users/ann"),
            (Platform::Linux, "/home/ann"),
            (Platform::Windows, "C:\\Users\\ann"),
        ] {
            let i = Inputs { legacy_exists: true, ..with_home(platform, home) };
            let legacy = p(home).join(".lanthorn");
            let d = UserDirs::resolve(&i).unwrap();
            assert_eq!(roots(&d), (legacy.clone(), legacy.clone(), legacy.join("cache"), legacy.clone()), "{platform:?}");
        }
    }

    #[test]
    fn legacy_beats_xdg() {
        let i = Inputs {
            legacy_exists: true,
            xdg_data: Some(p("/x/data")),
            ..with_home(Platform::Linux, "/home/ann")
        };
        assert_eq!(UserDirs::resolve(&i).unwrap(), UserDirs::single("/home/ann/.lanthorn"));
    }

    #[test]
    fn user_dir_is_the_single_folder_layout_and_beats_legacy() {
        let i = Inputs {
            legacy_exists: true,
            explicit: Some(p("/srv/lt")),
            ..with_home(Platform::Linux, "/home/ann")
        };
        let d = UserDirs::resolve(&i).unwrap();
        assert_eq!(roots(&d), (p("/srv/lt"), p("/srv/lt"), p("/srv/lt/cache"), p("/srv/lt")));
    }

    #[test]
    fn user_dir_needs_no_home() {
        let i = Inputs { explicit: Some(p("/srv/lt")), ..Inputs::empty(Platform::Linux) };
        assert_eq!(UserDirs::resolve(&i).unwrap(), UserDirs::single("/srv/lt"));
    }

    #[test]
    fn a_host_can_supply_all_four() {
        let d = UserDirs::new("/a", "/b", "/c", "/d");
        assert_eq!(
            (d.config(), d.data(), d.cache(), d.logs()),
            (Path::new("/a"), Path::new("/b"), Path::new("/c"), Path::new("/d"))
        );
    }

    #[test]
    fn the_single_layout_keeps_logs_at_base() {
        assert_eq!(UserDirs::single("/srv/lt").logs(), Path::new("/srv/lt"));
    }

    #[test]
    fn home_dir_is_never_empty() {
        if let Some(h) = home_dir() {
            assert!(!h.as_os_str().is_empty());
        }
    }
}
