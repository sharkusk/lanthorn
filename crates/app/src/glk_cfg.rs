//! Windows Glk per-game config (`<story stem>.cfg`) — SQ-1703 phase P1.
//!
//! Some Glulx stories ship a `.cfg` beside the story that tells Windows
//! Glulxe/Windows Glk how big to make its window and whether to draw window
//! borders (Photopia 2.01's `photo201.cfg`, Narcolepsy's `narco.cfg`). We parse
//! it into one value, [`GlkDesign`], carried on `AppState::glk_design`, so every
//! later phase (a pixel-exact Glk screen, the TUI's stretch, the mask) and every
//! host (TUI or web) reads the design from one place.
//!
//! **Where the key meanings come from** (documentation, not source): Windows
//! Glulxe's help page `Win/help/config.htm`
//! (<https://github.com/DavidKinder/Windows-Glulxe/blob/master/Win/help/config.htm>,
//! David Kinder). It states: the file has the same name as the game with a
//! `.cfg` extension; each line is `Key=value` (e.g. `WindowBorders=no`); the
//! keys are `WindowBorders` (yes/no, borders between windows), `WindowFrame`
//! (yes/no, title bar and the border around the whole window), `WindowMask`
//! (a Blorb resource number whose graphic masks the window — white is
//! transparent — only with `WindowFrame=no`), `WindowWidth`/`WindowHeight`
//! (the interpreter window is sized so a single full-size Glk window has this
//! many pixels), `FullScreen` (yes/no), `FontName`, `FixedFontName`,
//! `FontSize` (points) and `FontFile`. The page gives no defaults.
//!
//! **Not stated by the documentation, so UNVERIFIED here**: whether key names
//! are case-insensitive, whether whitespace around `=` is allowed, whether
//! comments exist, and any value spelling beyond `yes`/`no`. We are lenient on
//! the first two (the specimens use exactly the documented spelling), ignore
//! any line that is not `key=value`, and read a boolean only as `yes`/`no`
//! (anything else leaves the key unset). Unknown keys are ignored.
//!
//! Discovery is the exact `<stem>.cfg` beside the story, loose files only (a
//! `.cfg.txt` spelling is not accepted until a specimen shows one, and a story
//! launched out of a zip does not read an entry).

use std::path::Path;

use crate::garglk_ini::GarglkOverlay;

/// A story's Windows Glk `.cfg`, every key optional (an absent key is `None`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlkDesign {
    /// `WindowWidth` — pixel width of a single full-size Glk window.
    pub window_width: Option<u32>,
    /// `WindowHeight` — pixel height of a single full-size Glk window.
    pub window_height: Option<u32>,
    /// `WindowBorders` — borders between windows (`yes` = shown).
    pub borders: Option<bool>,
    /// `WindowFrame` — the interpreter window's title bar and outer border.
    pub frame: Option<bool>,
    /// `WindowMask` — Blorb resource number of the window-shape mask picture.
    pub mask_pict: Option<u32>,
    /// `FontName` — proportional font.
    pub font_name: Option<String>,
    /// `FixedFontName` — fixed-width font.
    pub fixed_font_name: Option<String>,
    /// `FontSize` — points.
    pub font_size: Option<u32>,
    /// `FontFile` — a font file to load fonts from.
    pub font_file: Option<String>,
    /// `FullScreen`.
    pub fullscreen: Option<bool>,
}

impl GlkDesign {
    /// The design size in pixels, when the `.cfg` gave both dimensions (and
    /// neither is zero).
    pub fn size(&self) -> Option<(u32, u32)> {
        match (self.window_width, self.window_height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        }
    }

    /// The borderless preference this `.cfg` states: `WindowBorders=no` is
    /// borderless (`Some(true)`), `yes` is bordered (`Some(false)`).
    pub fn borderless(&self) -> Option<bool> {
        self.borders.map(|b| !b)
    }

    /// Parse `.cfg` text. Never fails: a line that is not `key=value`, an
    /// unknown key or an unreadable value is skipped.
    pub fn parse(text: &str) -> GlkDesign {
        let mut d = GlkDesign::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            let num = || value.parse::<u32>().ok();
            let yes_no = || match value.to_ascii_lowercase().as_str() {
                "yes" => Some(true),
                "no" => Some(false),
                _ => None,
            };
            let text = || (!value.is_empty()).then(|| value.to_string());
            match key.as_str() {
                "windowwidth" => d.window_width = num(),
                "windowheight" => d.window_height = num(),
                "windowborders" => d.borders = yes_no(),
                "windowframe" => d.frame = yes_no(),
                "windowmask" => d.mask_pict = num(),
                "fontname" => d.font_name = text(),
                "fixedfontname" => d.fixed_font_name = text(),
                "fontsize" => d.font_size = num(),
                "fontfile" => d.font_file = text(),
                "fullscreen" => d.fullscreen = yes_no(),
                _ => {}
            }
        }
        d
    }
}

/// Discover `<story stem>.cfg` beside `story_path` (exact `.cfg` only). `None`
/// when there is no such file or it cannot be read.
pub fn discover(story_path: &Path) -> Option<GlkDesign> {
    let dir = story_path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let stem = story_path.file_stem()?.to_string_lossy().into_owned();
    let cand = dir.join(format!("{stem}.cfg"));
    if !cand.is_file() {
        return None;
    }
    let bytes = std::fs::read(&cand).ok()?;
    Some(GlkDesign::parse(&String::from_utf8_lossy(&bytes)))
}

/// The one place the Glulx borderless-windows preference is resolved (boot,
/// `@restart` and the settings screen all call it), most specific first:
///
/// 1. the player's per-game `config.toml` `borderless_windows` (`per_game`);
/// 2. garglk.ini's `wborderx`/`wbordery` (`GarglkOverlay::borderless`);
/// 3. this story's `.cfg` `WindowBorders` ([`GlkDesign::borderless`]);
/// 4. bordered.
pub fn resolve_borderless(
    per_game: Option<bool>,
    garglk: Option<&GarglkOverlay>,
    design: Option<&GlkDesign>,
) -> bool {
    per_game
        .or_else(|| garglk.and_then(|o| o.borderless))
        .or_else(|| design.and_then(GlkDesign::borderless))
        .unwrap_or(false)
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_key() {
        let d = GlkDesign::parse(
            "WindowBorders=no\nWindowFrame=yes\nWindowWidth=800\nWindowHeight=600\n\
             WindowMask=3\nFontName=Tahoma\nFixedFontName=Courier New\nFontSize=10\n\
             FontFile=a.ttf\nFullScreen=NO\n",
        );
        assert_eq!(d.size(), Some((800, 600)));
        assert_eq!(d.borders, Some(false));
        assert_eq!(d.frame, Some(true));
        assert_eq!(d.mask_pict, Some(3));
        assert_eq!(d.font_name.as_deref(), Some("Tahoma"));
        assert_eq!(d.fixed_font_name.as_deref(), Some("Courier New"));
        assert_eq!(d.font_size, Some(10));
        assert_eq!(d.font_file.as_deref(), Some("a.ttf"));
        assert_eq!(d.fullscreen, Some(false));
        assert_eq!(d.borderless(), Some(true));
    }

    #[test]
    fn tolerates_whitespace_case_crlf_and_unknown_keys() {
        let d = GlkDesign::parse("  windowwidth = 640 \r\nWINDOWHEIGHT=480\r\nBogus=1\nnot a pair\n\n");
        assert_eq!(d.size(), Some((640, 480)));
        assert_eq!(d, GlkDesign { window_width: Some(640), window_height: Some(480), ..Default::default() });
    }

    #[test]
    fn bad_values_leave_the_key_unset() {
        let d = GlkDesign::parse("WindowWidth=wide\nWindowHeight=-4\nWindowBorders=maybe\nWindowMask=\nFontName=\n");
        assert_eq!(d, GlkDesign::default());
        assert_eq!(d.size(), None);
        assert_eq!(GlkDesign::parse("WindowWidth=640\n").size(), None);
        assert_eq!(GlkDesign::parse("WindowWidth=0\nWindowHeight=480").size(), None);
    }

    #[test]
    fn later_line_wins() {
        assert_eq!(GlkDesign::parse("WindowBorders=no\nWindowBorders=yes").borders, Some(true));
    }

    fn specimen(name: &str) -> Option<GlkDesign> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
        let text = std::fs::read_to_string(p).ok()?;
        Some(GlkDesign::parse(&text))
    }

    #[test]
    fn photopia_specimen() {
        let Some(d) = specimen("photo201.cfg") else { return };
        assert_eq!(d.size(), Some((640, 480)));
        assert_eq!((d.borders, d.frame, d.mask_pict), (Some(false), Some(false), Some(39)));
        assert_eq!((d.font_name.as_deref(), d.font_size), (Some("Tahoma"), Some(10)));
    }

    #[test]
    fn narcolepsy_specimen() {
        let Some(d) = specimen("narco.cfg") else { return };
        assert_eq!(d.size(), Some((800, 600)));
        assert_eq!((d.borders, d.frame, d.mask_pict), (Some(false), Some(false), Some(3)));
        assert_eq!((d.font_name.as_deref(), d.font_size), (Some("Tahoma"), Some(10)));
    }

    #[test]
    fn discovery_finds_stem_cfg_only() {
        let dir = crate::scratch_dir("glk-cfg-discover");
        let story = dir.join("tale.ulx");
        assert_eq!(discover(&story), None);
        std::fs::write(dir.join("tale.cfg.txt"), "WindowWidth=1\nWindowHeight=1\n").unwrap();
        assert_eq!(discover(&story), None, ".cfg.txt is not accepted");
        std::fs::write(dir.join("other.cfg"), "WindowWidth=1\nWindowHeight=1\n").unwrap();
        assert_eq!(discover(&story), None, "another story's cfg is not ours");
        std::fs::write(dir.join("tale.cfg"), "WindowWidth=320\nWindowHeight=200\n").unwrap();
        assert_eq!(discover(&story).and_then(|d| d.size()), Some((320, 200)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn borderless_order_per_game_then_garglk_then_cfg() {
        let no_borders = GlkDesign { borders: Some(false), ..Default::default() };
        let garglk = |b| GarglkOverlay { borderless: Some(b), ..GarglkOverlay::default() };
        // cfg alone decides when nothing outranks it.
        assert!(resolve_borderless(None, None, Some(&no_borders)));
        assert!(resolve_borderless(None, Some(&GarglkOverlay::default()), Some(&no_borders)));
        // garglk wborder wins over the cfg, in both directions.
        assert!(!resolve_borderless(None, Some(&garglk(false)), Some(&no_borders)));
        let borders = GlkDesign { borders: Some(true), ..Default::default() };
        assert!(resolve_borderless(None, Some(&garglk(true)), Some(&borders)));
        // the player's per-game choice wins over both.
        assert!(!resolve_borderless(Some(false), Some(&garglk(true)), Some(&no_borders)));
        assert!(resolve_borderless(Some(true), None, None));
        // nothing says anything: bordered.
        assert!(!resolve_borderless(None, None, None));
    }
}
