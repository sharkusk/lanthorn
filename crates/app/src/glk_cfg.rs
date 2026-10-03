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

// ── Stretch ("hybrid") design mode — SQ-1703 phase P3 ───────────────────────
//
// A story with a design size lays out at that size in design pixels and the
// whole frame is STRETCHED over the story pane: no letterbox, aspect not
// honoured. Everything a host needs for that lives here so the TUI and the web
// host share one set of arithmetic.

/// The Glk screen a story is laid out on in stretch mode: `design_px` design
/// pixels, with a text cell of the terminal's own cell divided by the per-axis
/// stretch (`sx = pane_px.0 / design_px.0`, `sy = pane_px.1 / design_px.1`),
/// so it is fractional and non-square. `pane_px` is the story pane in device
/// pixels (cells x `char_px`); `char_px` is one terminal cell in device pixels.
/// Re-derive it on every boot, pane resize, `char_px` change and `@restart`.
pub fn glk_design_screen(
    design_px: (u32, u32),
    pane_px: (f64, f64),
    char_px: (f64, f64),
) -> gvm::glk::GlkScreen {
    let sx = (pane_px.0 / design_px.0.max(1) as f64).max(1e-9);
    let sy = (pane_px.1 / design_px.1.max(1) as f64).max(1e-9);
    gvm::glk::GlkScreen::design(design_px, (char_px.0 / sx, char_px.1 / sy))
}

/// Map one design-pixel EDGE (`0..=design`) on an axis to a terminal-cell edge
/// (`0..=cells`), rounding half up in exact integer arithmetic. THE one rule:
/// apply it to BOTH edges of every window and neighbouring windows share an
/// edge exactly (no gap, no overlap), `edge(0) == 0` and `edge(design) ==
/// cells`, so the windows cover the pane. A window's cell extent is
/// `edge(far) - edge(near)`, never `round(width)` (the v6 ceil-vs-round trap).
pub fn design_px_to_cell_edge(px: u32, design: u32, cells: u32) -> u32 {
    let design = design.max(1) as u64;
    let px = (px as u64).min(design);
    ((px * cells as u64 * 2 + design) / (design * 2)) as u32
}

/// The inverse of the stretch, for a click: a position `rel` whole cells plus
/// `frac` (0..1) of a cell into a window drawn `window_cells` cells wide (or
/// tall) maps back to the design-pixel offset inside that window, whose canvas
/// is `window_px` design pixels on that axis and was stretched over exactly
/// those cells. Always `< window_px` (a window one pixel wide hears 0).
pub fn cell_offset_to_design_px(rel: u32, frac: f64, window_cells: u32, window_px: u32) -> u32 {
    if window_cells == 0 || window_px == 0 {
        return 0;
    }
    let t = ((rel as f64 + frac.clamp(0.0, 1.0)) * window_px as f64 / window_cells as f64).floor();
    (t as u32).min(window_px - 1)
}

/// A design-pixel rect `(left, top, width, height)` as the terminal-cell rect
/// `(left, top, width, height)` it covers, via [`design_px_to_cell_edge`] on
/// both edges of each axis. `cells` is the pane `(cols, rows)`.
pub fn design_rect_to_cells(
    rect: (u32, u32, u32, u32),
    design: (u32, u32),
    cells: (u32, u32),
) -> (u32, u32, u32, u32) {
    let x0 = design_px_to_cell_edge(rect.0, design.0, cells.0);
    let x1 = design_px_to_cell_edge(rect.0 + rect.2, design.0, cells.0);
    let y0 = design_px_to_cell_edge(rect.1, design.1, cells.1);
    let y1 = design_px_to_cell_edge(rect.1 + rect.3, design.1, cells.1);
    (x0, y0, x1 - x0, y1 - y0)
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

    #[test]
    fn design_screen_is_the_cell_over_the_stretch() {
        // 100x37 cells of 8x16: pane 800x592 over a 640x480 design.
        let s = glk_design_screen((640, 480), (800.0, 592.0), (8.0, 16.0));
        assert_eq!((s.size, s.unit_px), ((640, 480), (1, 1)));
        assert!((s.text_cell.0 - 6.4).abs() < 1e-9, "{:?}", s.text_cell);
        assert!((s.text_cell.1 - 480.0 / 37.0).abs() < 1e-9, "{:?}", s.text_cell);
        // A wide, short pane stretches x and y by very different factors.
        let w = glk_design_screen((640, 480), (1600.0, 320.0), (8.0, 16.0));
        assert!((w.text_cell.0 - 3.2).abs() < 1e-9 && (w.text_cell.1 - 24.0).abs() < 1e-9, "{:?}", w.text_cell);
        // The cell size cancels: only cells per design extent matters.
        let d = glk_design_screen((640, 480), (1600.0, 1184.0), (16.0, 32.0));
        assert!((d.text_cell.0 - 6.4).abs() < 1e-9);
    }

    #[test]
    fn edges_share_cover_and_never_disagree_with_what_the_story_is_told() {
        for &(w, h, splits_x, splits_y) in &[
            (640u32, 480u32, [14u32, 626], [58u32, 401]),
            (800, 600, [400, 480], [60, 438]),
        ] {
            for cols in 10u32..=250 {
                for rows in 5u32..=80 {
                    assert_eq!(design_px_to_cell_edge(0, w, cols), 0);
                    assert_eq!(design_px_to_cell_edge(w, w, cols), cols);
                    let screen = glk_design_screen((w, h), (cols as f64 * 8.0, rows as f64 * 16.0), (8.0, 16.0));
                    // Three abutting columns/rows tile the pane exactly.
                    let xs = [0, splits_x[0], splits_x[1], w];
                    let ys = [0, splits_y[0], splits_y[1], h];
                    let mut next = (0, 0);
                    for i in 0..3 {
                        let r = design_rect_to_cells((xs[i], ys[i], xs[i + 1] - xs[i], ys[i + 1] - ys[i]), (w, h), (cols, rows));
                        assert_eq!((r.0, r.1), next, "abut at {cols}x{rows}");
                        next = (r.0 + r.2, r.1 + r.3);
                        // chars told never exceed cells drawn, and fall short by at most one.
                        let told = (screen.chars_in(xs[i + 1] - xs[i], false), screen.chars_in(ys[i + 1] - ys[i], true));
                        assert!(told.0 <= r.2 && r.2 <= told.0 + 1, "x told {} drawn {} at {cols}", told.0, r.2);
                        assert!(told.1 <= r.3 && r.3 <= told.1 + 1, "y told {} drawn {} at {rows}", told.1, r.3);
                    }
                    assert_eq!(next, (cols, rows), "the windows cover the pane");
                }
            }
        }
    }
}
