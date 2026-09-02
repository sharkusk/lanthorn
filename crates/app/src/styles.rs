//! Per-game style overrides: a `style.toml` (and non-style `config.toml`
//! sidecar) stored in the story's per-game save directory (`game_dir`,
//! `<data_base>/<story-key>.save/`), layered over the global style.toml. Keyed
//! by story filename, co-located with the story's saves/aux/glkvfs (SQ-0346).
//! See docs/superpowers/specs/2026-06-25-per-game-styles-design.md.

use std::path::{Path, PathBuf};

/// The per-game style file path: `<game_dir>/style.toml`.
pub fn per_game_style_path(game_dir: &Path) -> PathBuf {
    game_dir.join("style.toml")
}

/// The per-game NON-style config sidecar: `<game_dir>/config.toml` — i.e. inside
/// the story's SAVE directory, beside its saves and aux data, not beside the
/// story file. Holds per-game overrides that are not part of the style schema
/// (`honor_game_colours`, `borderless_windows`, `show_map`, `pictures`,
/// `interpreter_number`, `v6_pixel_lock`, `guidance`, `v6_render`,
/// `panel`, `return_probe`), kept separate from `style.toml` so the style parser/writer
/// stays a pure style document.
///
/// Bare lines, never templated: an absent key means "inherit the global config",
/// so a file that listed every key with its default could not express that. See
/// [`PerGameConfig`].
pub fn per_game_config_path(game_dir: &Path) -> PathBuf {
    game_dir.join("config.toml")
}

/// Every per-game override the sidecar can carry, as ONE value (SQ-1123).
///
/// It was six positional `Option`s threaded through a private writer and
/// repeated by each of six public setters, with a comment in the middle asking
/// every future writer to remember to pass the keys it was not itself setting —
/// a hand-maintained invariant across call sites, which is the shape this repo's
/// refactoring policy names outright. Adding the three keys the border controls
/// persist would have made it nine, edited in nine places. Read-modify-write of
/// one struct cannot forget a key.
///
/// `None` everywhere is not "the defaults": it is **no sidecar at all**, and
/// writing it deletes the file. Absent key = inherit the global config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerGameConfig {
    pub honor_game_colours: Option<bool>,
    pub borderless_windows: Option<bool>,
    pub show_map: Option<bool>,
    pub pictures: Option<String>,
    pub interpreter_number: Option<u8>,
    pub v6_pixel_lock: Option<bool>,
    /// Lanthorn's Guiding Light, for this story (SQ-1123).
    pub guidance: Option<bool>,
    /// The v6 render mode for this story, as its config-file spelling
    /// (`hybrid` / `raster` / `extended`) — SQ-1123.
    pub v6_render: Option<String>,
    /// Which panel opens with this story — command, inventory, or none
    /// (SQ-1123, widened to a three-state cycle by SQ-1237). `None` here means
    /// no override at all (inherit `[command_panel] auto_open`), which is a
    /// different thing from `Some(SidePanel::None)` (this story is pinned to
    /// neither panel).
    pub panel: Option<crate::state::SidePanel>,
    /// Whether the return probe runs for this story (SQ-0785).
    pub return_probe: Option<bool>,
}

impl PerGameConfig {
    /// Every key the sidecar can carry, in the order [`PerGameConfig::write`]
    /// emits them.
    ///
    /// **Derived from, not maintained alongside.** The global `config.toml`
    /// template used to name the per-game keys in a hand-written sentence, and
    /// it had gone stale — it still said "these three keys" long after
    /// `v6_pixel_lock` and `borderless_windows` had joined. A list in prose,
    /// sitting far from the code that decides what is per-game, goes stale
    /// silently by construction. `config_template` builds its sentence from
    /// this, and `write_emits_exactly_the_declared_keys` fails if the writer and
    /// this list ever disagree.
    pub const KEYS: &'static [&'static str] = &[
        "honor_game_colours",
        "borderless_windows",
        "show_map",
        "v6_pixel_lock",
        "guidance",
        "panel",
        "return_probe",
        "pictures",
        "v6_render",
        "interpreter_number",
    ];

    /// Read the sidecar. Every key absent when the file is missing or unparseable
    /// — a corrupt sidecar inherits the global config rather than failing a boot.
    pub fn read(game_dir: &Path) -> PerGameConfig {
        let Some(v) = std::fs::read_to_string(per_game_config_path(game_dir))
            .ok()
            .and_then(|t| t.parse::<toml::Value>().ok())
        else {
            return PerGameConfig::default();
        };
        let b = |k: &str| v.get(k).and_then(|x| x.as_bool());
        let s = |k: &str| {
            v.get(k).and_then(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty())
                .map(str::to_string)
        };
        PerGameConfig {
            honor_game_colours: b("honor_game_colours"),
            borderless_windows: b("borderless_windows"),
            show_map: b("show_map"),
            pictures: s("pictures"),
            interpreter_number: v
                .get("interpreter_number")
                .and_then(|x| x.as_integer())
                .and_then(|n| u8::try_from(n).ok()),
            v6_pixel_lock: b("v6_pixel_lock"),
            guidance: b("guidance"),
            v6_render: s("v6_render"),
            panel: s("panel").as_deref().and_then(crate::state::SidePanel::from_key),
            return_probe: b("return_probe"),
        }
    }

    /// Write the sidecar, omitting every `None` key and DELETING the file when
    /// nothing is set. Creates `game_dir` if needed, so a click on turn one of a
    /// story that has never been saved writes the directory into existence.
    pub fn write(&self, game_dir: &Path) -> std::io::Result<()> {
        let path = per_game_config_path(game_dir);
        let mut body = String::new();
        fn put_bool(body: &mut String, k: &str, v: Option<bool>) {
            if let Some(v) = v {
                body.push_str(&format!("{k} = {v}\n"));
            }
        }
        put_bool(&mut body, "honor_game_colours", self.honor_game_colours);
        put_bool(&mut body, "borderless_windows", self.borderless_windows);
        put_bool(&mut body, "show_map", self.show_map);
        put_bool(&mut body, "v6_pixel_lock", self.v6_pixel_lock);
        put_bool(&mut body, "guidance", self.guidance);
        if let Some(p) = self.panel {
            body.push_str(&format!("panel = {}\n", toml::Value::String(p.key().to_string())));
        }
        put_bool(&mut body, "return_probe", self.return_probe);
        if let Some(v) = &self.pictures {
            body.push_str(&format!("pictures = {}\n", toml::Value::String(v.clone())));
        }
        if let Some(v) = &self.v6_render {
            body.push_str(&format!("v6_render = {}\n", toml::Value::String(v.clone())));
        }
        if let Some(v) = self.interpreter_number {
            body.push_str(&format!("interpreter_number = {v}\n"));
        }
        if body.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)
    }
}

/// Set one key and write the sidecar back, leaving every sibling key alone.
fn edit(game_dir: &Path, f: impl FnOnce(&mut PerGameConfig)) -> std::io::Result<()> {
    let mut cfg = PerGameConfig::read(game_dir);
    f(&mut cfg);
    cfg.write(game_dir)
}

/// Read the per-game `honor_game_colours` override, if the user set one.
/// `None` = no override (fall back to garglk.ini, then the global config default).
pub fn read_per_game_honor(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).honor_game_colours
}

/// Read the per-game `borderless_windows` override, if the user set one. `None`
/// = no override (fall back to the default: honor the Glk border hint). When
/// `Some(true)`, all window splits abut with no reserved gutter (SQ-0341).
pub fn read_per_game_borderless(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).borderless_windows
}

/// Read the per-game `show_map` override, if the user set one. `None` = no
/// override (fall back to the default: the map panel is shown). When
/// `Some(false)` the map panel starts hidden for this story (SQ-0304).
pub fn read_per_game_show_map(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).show_map
}

/// Read the per-game `pictures` override — SQ-0734 tier 3, the user naming a
/// native Infocom picture archive (`Pic.data`, `.MG1`/`.EG1`/`.CG1`) and thereby
/// ASSERTING that it belongs to this story. `None` = no override, so the Blorb
/// (tier 1) or the disk image the story was mounted from (tier 2) decides.
///
/// The value is a path: absolute, or relative to the STORY's own directory —
/// "beside the story" is where these archives sit. Resolution and validation
/// live in [`crate::graphics::PictureOverride::resolve`]; this only reads the key.
pub fn read_per_game_pictures(game_dir: &Path) -> Option<String> {
    PerGameConfig::read(game_dir).pictures
}

/// Read the per-game `interpreter_number` override — the machine this one story
/// presents itself as, ZMSD §11.1.3 (SQ-0789). `None` = no override, so the
/// launch's own precedence decides: a CLI number, else the flavour of a named
/// picture archive, else the medium, else Frotz's rule.
pub fn read_per_game_interpreter_number(game_dir: &Path) -> Option<u8> {
    PerGameConfig::read(game_dir).interpreter_number
}

/// Read the per-game `v6_pixel_lock` override, if the user set one (SQ-0945).
/// `None` = no override, so the global `v6_pixel_lock` decides.
///
/// Which rung of the magnification ladder a story's artwork looks right on is a
/// fact about that story's own press — the density its archive declares is what
/// [`crate::render::v6_layout::scale_ladder_step`] derives the ladder from — so
/// the switch is per-game before it is global. Written by `set-v6-pixel-lock`.
pub fn read_per_game_v6_pixel_lock(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).v6_pixel_lock
}

/// Read the per-game `guidance` override (SQ-1123). `None` = no override, so the
/// global `guidance` decides.
///
/// Whether you want Lanthorn's Guiding Light is a standing preference about how
/// you want to play a PARTICULAR story — off for the one you know by heart, on
/// for the one you have just opened — so it is per-game before it is global.
pub fn read_per_game_guidance(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).guidance
}

/// Read the per-game `v6_render` override (SQ-1123), as its config spelling.
/// `None` = no override, so the global `v6_render` decides.
///
/// Raster began as a FALLBACK — the mode you escaped to when hybrid could not
/// cope — and a temporary escape hatch rightly did not persist. It is a
/// destination now, with `extended` beside it, and a player may genuinely prefer
/// raster for one game and hybrid for another. That makes the mode a property of
/// the story, exactly as `v6_pixel_lock` already is.
pub fn read_per_game_v6_render(game_dir: &Path) -> Option<String> {
    PerGameConfig::read(game_dir).v6_render
}

/// Read the per-game `panel` override (SQ-1123, widened to three states by
/// SQ-1237). `None` = no override, so the global `[command_panel] auto_open`
/// decides whether the command panel opens with the story (the inventory panel
/// has no global auto-open of its own).
pub fn read_per_game_panel(game_dir: &Path) -> Option<crate::state::SidePanel> {
    PerGameConfig::read(game_dir).panel
}

/// Read the per-game `return_probe` override (SQ-0785). `None` = no override, so
/// the global `return_probe` decides.
///
/// Per-game before it is global for the same reason the pixel lock is: how much
/// silent work a particular story is worth is a fact about that story. A small
/// Z-machine game answers a probe in milliseconds; a large Glulx one takes long
/// enough that a player may want it on for the first and off for the second.
pub fn read_per_game_return_probe(game_dir: &Path) -> Option<bool> {
    PerGameConfig::read(game_dir).return_probe
}

/// Persist (or clear) the per-game `return_probe` override, preserving every
/// sibling key (SQ-0785).
pub fn write_per_game_return_probe(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.return_probe = value)
}

/// Persist (or clear) the per-game `honor_game_colours` override, preserving
/// every sibling key. `Some(v)` writes it; `None` clears it (→ fall back to
/// garglk.ini / the global default).
pub fn write_per_game_honor(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.honor_game_colours = value)
}

/// Persist (or clear) the per-game `borderless_windows` override, preserving
/// every sibling key (SQ-0341).
pub fn write_per_game_borderless(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.borderless_windows = value)
}

/// Persist (or clear) the per-game `show_map` override, preserving every sibling
/// key (SQ-0304).
pub fn write_per_game_show_map(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.show_map = value)
}

/// Persist (or clear) the per-game `pictures` override — the launch-options
/// dialog's "save as this game's default" checkbox (SQ-0789), preserving every
/// sibling key.
///
/// `Some(name)` names an archive beside the story; `None` clears the key, which
/// is what "inherit" means and is NOT the same as writing the global default.
pub fn write_per_game_pictures(game_dir: &Path, value: Option<String>) -> std::io::Result<()> {
    edit(game_dir, |c| c.pictures = value)
}

/// Persist (or clear) the per-game `interpreter_number` override (SQ-0789),
/// preserving every sibling key. `None` clears it back to inheriting the
/// launch's own precedence.
pub fn write_per_game_interpreter_number(game_dir: &Path, value: Option<u8>) -> std::io::Result<()> {
    edit(game_dir, |c| c.interpreter_number = value)
}

/// Persist (or clear) the per-game `v6_pixel_lock` override (SQ-0945),
/// preserving every sibling key. `None` clears it back to inheriting the global
/// `v6_pixel_lock`, which is NOT the same as writing the global value down.
pub fn write_per_game_v6_pixel_lock(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.v6_pixel_lock = value)
}

/// Persist (or clear) the per-game `guidance` override (SQ-1123), preserving
/// every sibling key. `None` clears it back to inheriting the global setting.
pub fn write_per_game_guidance(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    edit(game_dir, |c| c.guidance = value)
}

/// Persist (or clear) the per-game `v6_render` override (SQ-1123), preserving
/// every sibling key. `None` clears it back to inheriting the global mode.
pub fn write_per_game_v6_render(game_dir: &Path, value: Option<String>) -> std::io::Result<()> {
    edit(game_dir, |c| c.v6_render = value)
}

/// Persist (or clear) the per-game `panel` override (SQ-1123, widened by
/// SQ-1237), preserving every sibling key. `None` clears it back to inheriting
/// `[command_panel] auto_open`.
pub fn write_per_game_panel(
    game_dir: &Path,
    value: Option<crate::state::SidePanel>,
) -> std::io::Result<()> {
    edit(game_dir, |c| c.panel = value)
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(&format!("pgstyle-{tag}"))
    }

    #[test]
    fn per_game_style_path_is_game_dir_style_toml() {
        let dir = tmp("path");
        assert_eq!(per_game_style_path(&dir), dir.join("style.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_honor_roundtrips_and_clears() {
        let dir = tmp("honor");
        // Absent → no override.
        assert_eq!(read_per_game_honor(&dir), None);
        // Some(false) persists and reads back.
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(false));
        assert!(per_game_config_path(&dir).is_file());
        // Overwrite with Some(true).
        write_per_game_honor(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(true));
        // None (auto) clears the override.
        write_per_game_honor(&dir, None).unwrap();
        assert_eq!(read_per_game_honor(&dir), None);
        assert!(!per_game_config_path(&dir).exists());
        // Clearing when already absent is a no-op, not an error.
        write_per_game_honor(&dir, None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_borderless_roundtrips_and_coexists_with_honor() {
        let dir = tmp("borderless");
        assert_eq!(read_per_game_borderless(&dir), None);
        // Borderless persists and reads back.
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_borderless(&dir), Some(true));
        // Setting honor must PRESERVE the borderless override (shared sidecar).
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(false));
        assert_eq!(read_per_game_borderless(&dir), Some(true), "honor write kept borderless");
        // Clearing borderless keeps honor.
        write_per_game_borderless(&dir, None).unwrap();
        assert_eq!(read_per_game_borderless(&dir), None);
        assert_eq!(read_per_game_honor(&dir), Some(false), "borderless clear kept honor");
        // Clearing the last key removes the sidecar.
        write_per_game_honor(&dir, None).unwrap();
        assert!(!per_game_config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The declared key list and the writer must agree, because the global
    /// `config.toml` template tells the player what a per-game file may hold and
    /// derives that sentence from [`PerGameConfig::KEYS`]. A key the writer
    /// emits but the list omits is a setting nobody is told about; a key the
    /// list names but the writer never emits is a promise nothing keeps.
    #[test]
    fn write_emits_exactly_the_declared_keys() {
        let dir = tmp("keys");
        let every = PerGameConfig {
            honor_game_colours: Some(true),
            borderless_windows: Some(true),
            show_map: Some(true),
            pictures: Some("Pic.data".into()),
            interpreter_number: Some(6),
            v6_pixel_lock: Some(true),
            guidance: Some(true),
            v6_render: Some("raster".into()),
            panel: Some(crate::state::SidePanel::Command),
            return_probe: Some(true),
        };
        every.write(&dir).unwrap();
        let text = std::fs::read_to_string(per_game_config_path(&dir)).unwrap();
        let written: Vec<&str> =
            text.lines().filter_map(|l| l.split_once(" = ")).map(|(k, _)| k).collect();
        assert_eq!(written, PerGameConfig::KEYS, "the writer and the declared list disagree");
        // …and every one of them reads back, so the list is not merely spelled
        // the same as what the writer prints.
        assert_eq!(PerGameConfig::read(&dir), every);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-1123: the three keys the border controls added persist, coexist with
    /// every older key, and clear back to "inherit the global setting".
    #[test]
    fn the_border_controls_keys_roundtrip_and_coexist() {
        let dir = tmp("controls");
        assert_eq!(read_per_game_guidance(&dir), None);
        assert_eq!(read_per_game_v6_render(&dir), None);
        assert_eq!(read_per_game_panel(&dir), None);

        write_per_game_guidance(&dir, Some(false)).unwrap();
        write_per_game_v6_render(&dir, Some("raster".into())).unwrap();
        write_per_game_panel(&dir, Some(crate::state::SidePanel::Command)).unwrap();
        // …alongside two keys that predate them, to prove the shared sidecar is
        // read-modify-written rather than rewritten from whatever the caller
        // remembered to pass.
        write_per_game_v6_pixel_lock(&dir, Some(true)).unwrap();
        write_per_game_pictures(&dir, Some("Pic.data".into())).unwrap();

        assert_eq!(read_per_game_guidance(&dir), Some(false));
        assert_eq!(read_per_game_v6_render(&dir).as_deref(), Some("raster"));
        assert_eq!(read_per_game_panel(&dir), Some(crate::state::SidePanel::Command));
        assert_eq!(read_per_game_v6_pixel_lock(&dir), Some(true));
        assert_eq!(read_per_game_pictures(&dir).as_deref(), Some("Pic.data"));

        // `auto` on one key clears exactly that key.
        write_per_game_v6_render(&dir, None).unwrap();
        assert_eq!(read_per_game_v6_render(&dir), None, "cleared");
        assert_eq!(read_per_game_guidance(&dir), Some(false), "and only that one");
        assert_eq!(read_per_game_v6_pixel_lock(&dir), Some(true));

        // Clearing the last key removes the sidecar entirely — absent means
        // inherit, which a file full of defaults could not say.
        for f in [
            write_per_game_guidance as fn(&Path, Option<bool>) -> std::io::Result<()>,
            write_per_game_v6_pixel_lock,
        ] {
            f(&dir, None).unwrap();
        }
        write_per_game_panel(&dir, None).unwrap();
        write_per_game_pictures(&dir, None).unwrap();
        assert!(!per_game_config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A click on turn one of a story that has never been saved must write the
    /// sidecar into existence rather than fail — `game_dir` may not exist yet.
    #[test]
    fn a_write_creates_the_game_dir_it_needs() {
        let dir = tmp("mkdir").join("never-saved.save");
        assert!(!dir.exists());
        write_per_game_guidance(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_guidance(&dir), Some(true));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// An unparseable sidecar inherits the global config rather than failing a
    /// boot, and an unrecognised `v6_render` token is not a mode.
    #[test]
    fn a_broken_sidecar_reads_as_no_overrides() {
        let dir = tmp("broken");
        std::fs::write(per_game_config_path(&dir), "this is not = = toml\n").unwrap();
        assert_eq!(PerGameConfig::read(&dir), PerGameConfig::default());
        std::fs::write(per_game_config_path(&dir), "v6_render = \"sepia\"\n").unwrap();
        assert_eq!(read_per_game_v6_render(&dir).as_deref(), Some("sepia"));
        assert_eq!(crate::config::v6_render_from_key("sepia"), None, "…and names no mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0734: `pictures` is hand-written and has no writer of its own, so the
    /// three keys that DO have writers must carry it through. Without that,
    /// toggling game colours from the UI would silently delete the picture
    /// archive the user chose and quietly revert the game to its Blorb art —
    /// the exact "plausible but wrong" failure the tier policy exists to avoid,
    /// caused by us rather than by a bad guess.
    #[test]
    fn a_hand_written_pictures_key_survives_every_sibling_write() {
        let dir = tmp("pictures");
        assert_eq!(read_per_game_pictures(&dir), None);
        std::fs::write(per_game_config_path(&dir), "pictures = \"FMVPOKER.EG1\"\n").unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()));

        // Each writer rewrites the whole sidecar; each must preserve `pictures`.
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "honor write");
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "borderless write");
        write_per_game_show_map(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "show_map write");

        // Clearing every OTHER key must not delete the file out from under it.
        write_per_game_honor(&dir, None).unwrap();
        write_per_game_borderless(&dir, None).unwrap();
        write_per_game_show_map(&dir, None).unwrap();
        assert!(per_game_config_path(&dir).is_file(), "pictures alone keeps the sidecar alive");
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()));

        // A blank value is not a choice.
        std::fs::write(per_game_config_path(&dir), "pictures = \"  \"\n").unwrap();
        assert_eq!(read_per_game_pictures(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0789: the two keys the launch-options dialog writes round-trip, clear
    /// back to absent (which is what "inherit" means — not "written at the
    /// default"), and survive every sibling's whole-file rewrite.
    #[test]
    fn the_launch_option_keys_roundtrip_and_coexist() {
        let dir = tmp("launchopts");
        assert_eq!(read_per_game_pictures(&dir), None);
        assert_eq!(read_per_game_interpreter_number(&dir), None);

        write_per_game_pictures(&dir, Some("zork0.mg1".into())).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("zork0.mg1".to_string()));
        write_per_game_interpreter_number(&dir, Some(4)).unwrap();
        assert_eq!(read_per_game_interpreter_number(&dir), Some(4));
        assert_eq!(read_per_game_pictures(&dir), Some("zork0.mg1".to_string()), "sibling preserved");

        // Every other writer carries both through.
        write_per_game_honor(&dir, Some(false)).unwrap();
        write_per_game_borderless(&dir, Some(true)).unwrap();
        write_per_game_show_map(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("zork0.mg1".to_string()));
        assert_eq!(read_per_game_interpreter_number(&dir), Some(4));

        // Clearing one leaves the other; clearing the last removes the file.
        write_per_game_pictures(&dir, None).unwrap();
        assert_eq!(read_per_game_pictures(&dir), None);
        assert_eq!(read_per_game_interpreter_number(&dir), Some(4));
        write_per_game_interpreter_number(&dir, None).unwrap();
        write_per_game_honor(&dir, None).unwrap();
        write_per_game_borderless(&dir, None).unwrap();
        write_per_game_show_map(&dir, None).unwrap();
        assert!(!per_game_config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_show_map_roundtrips_and_coexists_with_others() {
        let dir = tmp("show_map");
        assert_eq!(read_per_game_show_map(&dir), None);
        // show_map persists and reads back.
        write_per_game_show_map(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_show_map(&dir), Some(false));
        // Writing the other two keys must PRESERVE the show_map override.
        write_per_game_honor(&dir, Some(true)).unwrap();
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_show_map(&dir), Some(false), "sibling writes kept show_map");
        assert_eq!(read_per_game_honor(&dir), Some(true));
        assert_eq!(read_per_game_borderless(&dir), Some(true));
        // Clearing show_map keeps the siblings.
        write_per_game_show_map(&dir, None).unwrap();
        assert_eq!(read_per_game_show_map(&dir), None);
        assert_eq!(read_per_game_honor(&dir), Some(true), "show_map clear kept honor");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0945: `v6_pixel_lock` round-trips, clears back to ABSENT — which is
    /// "inherit the global key", not "written at the default" — and survives
    /// every sibling's whole-file rewrite, as every key in this sidecar must.
    #[test]
    fn per_game_v6_pixel_lock_roundtrips_and_coexists_with_others() {
        let dir = tmp("pixellock");
        assert_eq!(read_per_game_v6_pixel_lock(&dir), None);

        write_per_game_v6_pixel_lock(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_v6_pixel_lock(&dir), Some(true));
        write_per_game_v6_pixel_lock(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_v6_pixel_lock(&dir), Some(false));

        // `false` is a CHOICE here, not a default to elide: it says "free-scale
        // this one story" even when the global key is on. So it must be written.
        assert!(
            std::fs::read_to_string(per_game_config_path(&dir)).unwrap().contains("v6_pixel_lock = false"),
            "an explicit off is a per-game override and has to reach the file"
        );

        // Every other writer carries it through.
        write_per_game_honor(&dir, Some(true)).unwrap();
        write_per_game_borderless(&dir, Some(true)).unwrap();
        write_per_game_show_map(&dir, Some(false)).unwrap();
        write_per_game_pictures(&dir, Some("zork0.mg1".into())).unwrap();
        write_per_game_interpreter_number(&dir, Some(4)).unwrap();
        assert_eq!(read_per_game_v6_pixel_lock(&dir), Some(false), "sibling writes kept it");

        // And it carries THEM through.
        write_per_game_v6_pixel_lock(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(true));
        assert_eq!(read_per_game_borderless(&dir), Some(true));
        assert_eq!(read_per_game_show_map(&dir), Some(false));
        assert_eq!(read_per_game_pictures(&dir), Some("zork0.mg1".to_string()));
        assert_eq!(read_per_game_interpreter_number(&dir), Some(4));

        // Clearing it leaves the siblings; clearing the last removes the file.
        write_per_game_v6_pixel_lock(&dir, None).unwrap();
        assert_eq!(read_per_game_v6_pixel_lock(&dir), None);
        assert_eq!(read_per_game_honor(&dir), Some(true), "its clear kept honor");
        write_per_game_honor(&dir, None).unwrap();
        write_per_game_borderless(&dir, None).unwrap();
        write_per_game_show_map(&dir, None).unwrap();
        write_per_game_pictures(&dir, None).unwrap();
        write_per_game_interpreter_number(&dir, None).unwrap();
        assert!(!per_game_config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
