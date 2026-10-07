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
/// 4. the player's global `config.toml` `borderless_windows` (`user_default`, SQ-1740);
/// 5. bordered.
pub fn resolve_borderless(
    per_game: Option<bool>,
    garglk: Option<&GarglkOverlay>,
    design: Option<&GlkDesign>,
    user_default: Option<bool>,
) -> bool {
    per_game
        .or_else(|| garglk.and_then(|o| o.borderless))
        .or_else(|| design.and_then(GlkDesign::borderless))
        .or(user_default)
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

// ── Fit modes — SQ-1707 step A ─────────────────────────────────────────────
//
// A design-size story can fill the pane two ways: STRETCH it (per-axis scale,
// aspect not honoured — the default, and everything above) or fit it by ASPECT
// (one uniform scale, the largest frame that fits, centred, the leftover area
// painted like outside-the-mask). [`GlkFit`] is the ONE value both a terminal
// host (units are cells) and a pixel host (units are pixels) ask for the
// frame, each window's rectangle and the click-to-design-pixel inverse, so the
// two cannot disagree about where the frame is.

/// How a design-size story fills the story pane. `glk_design_fit` in
/// `config.toml` (global) and the per-game sidecar (per-game wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GlkFitMode {
    /// The frame fills the pane, each axis scaled on its own (default).
    #[default]
    Stretch,
    /// One uniform scale: the largest frame that fits, centred.
    Aspect,
}

impl GlkFitMode {
    /// The `glk_design_fit` token for this mode — one spelling for the file,
    /// the sidecar and the command.
    pub fn key(self) -> &'static str {
        match self {
            GlkFitMode::Stretch => "stretch",
            GlkFitMode::Aspect => "aspect",
        }
    }

    /// The mode a `glk_design_fit` token names, or `None` for anything else.
    pub fn from_key(token: &str) -> Option<GlkFitMode> {
        match token {
            "stretch" => Some(GlkFitMode::Stretch),
            "aspect" => Some(GlkFitMode::Aspect),
            _ => None,
        }
    }

    /// The other mode (what a bare `/set-glk-fit` steps to).
    pub fn toggled(self) -> GlkFitMode {
        match self {
            GlkFitMode::Stretch => GlkFitMode::Aspect,
            GlkFitMode::Aspect => GlkFitMode::Stretch,
        }
    }
}

/// The fit mode in force: the per-game sidecar's, else the global config's.
pub fn resolve_fit_mode(per_game: Option<GlkFitMode>, global: GlkFitMode) -> GlkFitMode {
    per_game.unwrap_or(global)
}

/// Whether design-size layout is on: the per-game `glk_design`, else the global
/// one (default on).
pub fn resolve_design_on(per_game: Option<bool>, global: bool) -> bool {
    per_game.unwrap_or(global)
}

/// A whole-unit rectangle in a [`GlkFit`]'s units (cells or pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FitRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl FitRect {
    /// Whether the point `(x, y)` is inside (right/bottom edges exclusive).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64 && y >= self.y as f64 && x < (self.x + self.w) as f64 && y < (self.y + self.h) as f64
    }
}

/// What a [`GlkFit`]'s pane and results are measured in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlkFitUnit {
    /// Device pixels (a pixel host). Results are exact pixels.
    Pixels,
    /// Terminal cells, each `cell_px` device pixels (the TUI). The aspect is
    /// computed in device pixels (cells are not square) and the frame is then
    /// snapped to whole cells; see [`GlkFit::frame`].
    Cells { cell_px: (f64, f64) },
}

/// Mode + design size + pane + unit: everything needed to place a design-size
/// story in a pane, for the TUI (cells) and a pixel host (pixels) alike.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlkFit {
    pub mode: GlkFitMode,
    /// The `.cfg` design size in design pixels.
    pub design: (u32, u32),
    /// The pane in this fit's unit.
    pub pane: (u32, u32),
    pub unit: GlkFitUnit,
}

fn round_div(n: u64, d: u64) -> u64 {
    (n * 2 + d) / (d * 2)
}

impl GlkFit {
    /// A fit for a host that draws in device pixels; `pane_px` is its story pane.
    pub fn pixels(mode: GlkFitMode, design: (u32, u32), pane_px: (u32, u32)) -> GlkFit {
        GlkFit { mode, design, pane: pane_px, unit: GlkFitUnit::Pixels }
    }

    /// A fit for a cell host; `pane_cells` is `(cols, rows)`, `cell_px` one cell
    /// in device pixels.
    pub fn cells(mode: GlkFitMode, design: (u32, u32), pane_cells: (u32, u32), cell_px: (f64, f64)) -> GlkFit {
        GlkFit { mode, design, pane: pane_cells, unit: GlkFitUnit::Cells { cell_px } }
    }

    fn design_nz(&self) -> (u32, u32) {
        (self.design.0.max(1), self.design.1.max(1))
    }

    /// The frame's size and offset in this fit's unit.
    ///
    /// **Stretch**: the whole pane. **Aspect**: one uniform scale, the largest
    /// frame of the design's aspect ratio that fits, centred.
    ///
    /// In **pixels** the limiting axis is the pane's own extent and the other
    /// is `round(design * pane / design_limiting)`, clamped to the pane; the
    /// offset is `floor(leftover / 2)`.
    ///
    /// In **cells** the scale is computed in DEVICE pixels first (`s = min(
    /// pane_px.w / design.w, pane_px.h / design.h)`, cells being non-square),
    /// then each side is snapped to whole cells: `round(design * s / cell)`,
    /// clamped to `1..=pane`, so the limiting axis fills the pane exactly and
    /// the other is within half a cell of the true aspect. The offset is
    /// `floor(leftover / 2)` cells, so frame plus [`Self::letterbox`] tile the
    /// pane with no gap or overlap (the odd leftover cell goes right/bottom).
    pub fn frame(&self) -> FitRect {
        let (pw, ph) = self.pane;
        let full = FitRect { x: 0, y: 0, w: pw, h: ph };
        if self.mode == GlkFitMode::Stretch || pw == 0 || ph == 0 {
            return full;
        }
        let (dw, dh) = self.design_nz();
        let (w, h) = match self.unit {
            GlkFitUnit::Pixels => {
                if pw as u64 * dh as u64 <= ph as u64 * dw as u64 {
                    (pw, (round_div(dh as u64 * pw as u64, dw as u64) as u32).clamp(1, ph))
                } else {
                    ((round_div(dw as u64 * ph as u64, dh as u64) as u32).clamp(1, pw), ph)
                }
            }
            GlkFitUnit::Cells { cell_px } => {
                let (cw, ch) = (cell_px.0.max(1e-9), cell_px.1.max(1e-9));
                let s = (pw as f64 * cw / dw as f64).min(ph as f64 * ch / dh as f64);
                (
                    ((dw as f64 * s / cw).round() as u32).clamp(1, pw),
                    ((dh as f64 * s / ch).round() as u32).clamp(1, ph),
                )
            }
        };
        FitRect { x: (pw - w) / 2, y: (ph - h) / 2, w, h }
    }

    /// The area outside the frame as up to four non-overlapping rects (top and
    /// bottom bands full width, left and right bands beside the frame); empty
    /// when the frame is the pane. Frame + these tile the pane exactly.
    pub fn letterbox(&self) -> Vec<FitRect> {
        let f = self.frame();
        let (pw, ph) = self.pane;
        [
            FitRect { x: 0, y: 0, w: pw, h: f.y },
            FitRect { x: 0, y: f.y + f.h, w: pw, h: ph - (f.y + f.h) },
            FitRect { x: 0, y: f.y, w: f.x, h: f.h },
            FitRect { x: f.x + f.w, y: f.y, w: pw - (f.x + f.w), h: f.h },
        ]
        .into_iter()
        .filter(|r| r.w > 0 && r.h > 0)
        .collect()
    }

    /// The frame's size in DEVICE pixels (cells times the cell size in a cell
    /// fit).
    pub fn frame_px(&self) -> (f64, f64) {
        let f = self.frame();
        match self.unit {
            GlkFitUnit::Pixels => (f.w as f64, f.h as f64),
            GlkFitUnit::Cells { cell_px } => (f.w as f64 * cell_px.0, f.h as f64 * cell_px.1),
        }
    }

    /// The Glk screen the story is told: the design frame with the text cell
    /// IMPLIED by the actual (snapped) frame, `char_px / (frame_px / design)`
    /// per axis. `char_px` is one text cell in device pixels (a cell fit's own
    /// `cell_px`, or the pixel host's font cell).
    pub fn design_screen(&self, char_px: (f64, f64)) -> gvm::glk::GlkScreen {
        glk_design_screen(self.design, self.frame_px(), char_px)
    }

    /// A design-pixel rect `(left, top, width, height)` as the rect it covers
    /// in this fit's unit (frame offset included), by the same edge rule as
    /// [`design_rect_to_cells`] over the frame, so neighbours share edges.
    pub fn window_rect(&self, rect: (u32, u32, u32, u32)) -> FitRect {
        let f = self.frame();
        let (x, y, w, h) = design_rect_to_cells(rect, self.design, (f.w, f.h));
        FitRect { x: f.x + x, y: f.y + y, w, h }
    }

    /// The inverse mapping: a point `(x, y)` in this fit's unit (fractions
    /// allowed — a cell plus how far into it) to the design pixel under it, or
    /// `None` in the letterbox (outside the frame).
    pub fn to_design(&self, x: f64, y: f64) -> Option<(u32, u32)> {
        let f = self.frame();
        if !f.contains(x, y) {
            return None;
        }
        let (dw, dh) = self.design_nz();
        let px = ((x - f.x as f64) * dw as f64 / f.w as f64).floor() as u32;
        let py = ((y - f.y as f64) * dh as f64 / f.h as f64).floor() as u32;
        Some((px.min(dw - 1), py.min(dh - 1)))
    }
}

/// The Windows Glk window-shape mask (`WindowMask=<pict>`), SQ-1703 P4, as
/// a coverage table. Windows Glulxe's `config.htm`: "If a particular pixel in
/// the graphic is white then the window is transparent at that point, else it
/// is opaque." Transparent shows what lies behind the window (here the pane
/// background). The mask is the design frame's shape, so it is stretched per
/// axis over whatever it is asked about, exactly as the frame is.
///
/// Holds a summed-area table so any cell's opaque coverage is O(1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlkMask {
    width: u32,
    height: u32,
    /// `(width + 1) * (height + 1)` prefix sums of opaque pixels.
    sums: Vec<u32>,
}

impl GlkMask {
    /// Build from per-pixel opacity, row-major, `width * height` long.
    pub fn from_opaque(width: u32, height: u32, opaque: &[bool]) -> Option<GlkMask> {
        if width == 0 || height == 0 || opaque.len() != (width as usize) * (height as usize) {
            return None;
        }
        let stride = width as usize + 1;
        let mut sums = vec![0u32; stride * (height as usize + 1)];
        for y in 0..height as usize {
            let mut run = 0u32;
            for x in 0..width as usize {
                run += opaque[y * width as usize + x] as u32;
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + run;
            }
        }
        Some(GlkMask { width, height, sums })
    }

    /// Build from a decoded Pict: a pixel is transparent when it is white
    /// (every channel 255) or has no alpha, else opaque.
    pub fn from_rgba(img: &image::RgbaImage) -> Option<GlkMask> {
        let opaque: Vec<bool> =
            img.pixels().map(|p| p.0[3] != 0 && !(p.0[0] == 255 && p.0[1] == 255 && p.0[2] == 255)).collect();
        GlkMask::from_opaque(img.width(), img.height(), &opaque)
    }

    /// The mask picture's size in its own pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn rect_sum(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
        let s = self.width as usize + 1;
        let at = |x: u32, y: u32| self.sums[y as usize * s + x as usize];
        at(x1, y1) + at(x0, y0) - at(x1, y0) - at(x0, y1)
    }

    /// Opaque and total mask pixels under cell `(cx, cy)` of a `cols x rows`
    /// grid stretched over the whole mask. Cell edges are `floor(i * size /
    /// cells)` on each axis, so neighbours share edges exactly; a cell
    /// narrower than a mask pixel still owns at least one.
    pub fn cell_coverage(&self, cx: u32, cy: u32, cols: u32, rows: u32) -> (u32, u32) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let edge = |i: u32, size: u32, n: u32| ((i.min(n) as u64 * size as u64) / n as u64) as u32;
        let (mut x0, mut x1) = (edge(cx, self.width, cols), edge(cx + 1, self.width, cols));
        let (mut y0, mut y1) = (edge(cy, self.height, rows), edge(cy + 1, self.height, rows));
        if x1 <= x0 {
            x0 = x0.min(self.width - 1);
            x1 = x0 + 1;
        }
        if y1 <= y0 {
            y0 = y0.min(self.height - 1);
            y1 = y0 + 1;
        }
        (self.rect_sum(x0, y0, x1, y1), (x1 - x0) * (y1 - y0))
    }

    /// Whether mask pixel `(x, y)` is opaque (the window is drawn there);
    /// outside the mask picture is not.
    pub fn opaque(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height && self.rect_sum(x, y, x + 1, y + 1) == 1
    }

    /// The whole mask as an alpha image for a host that applies it per pixel:
    /// 255 where [`Self::opaque`], 0 where transparent, at the mask's own size.
    pub fn alpha_image(&self) -> image::GrayImage {
        image::GrayImage::from_fn(self.width, self.height, |x, y| image::Luma([if self.opaque(x, y) { 255 } else { 0 }]))
    }

    /// Whether cell `(cx, cy)` is drawn: a cell with LESS than 50% of its area
    /// inside the mask is hidden (exactly half stays). THE rule — the TUI and
    /// any other host share it. Presentation only: what the story is told
    /// never depends on it.
    pub fn cell_visible(&self, cx: u32, cy: u32, cols: u32, rows: u32) -> bool {
        let (opaque, area) = self.cell_coverage(cx, cy, cols, rows);
        opaque as u64 * 2 >= area as u64
    }

    /// [`Self::cell_visible`] for a whole `cols x rows` pane, row-major.
    pub fn visible_cells(&self, cols: u32, rows: u32) -> Vec<bool> {
        (0..rows).flat_map(|y| (0..cols).map(move |x| self.cell_visible(x, y, cols, rows))).collect()
    }

    /// Clip a window's canvas to the mask: `canvas` is the window `rect`
    /// `(left, top, width, height)` in DESIGN pixels on a `design`-pixel
    /// frame; every pixel whose mask sample is transparent gets alpha 0.
    /// Returns `None` when the mask leaves the whole rect opaque.
    pub fn clip_canvas(
        &self,
        canvas: &image::RgbaImage,
        rect: (u32, u32, u32, u32),
        design: (u32, u32),
    ) -> Option<image::RgbaImage> {
        let (dw, dh) = (design.0.max(1) as u64, design.1.max(1) as u64);
        let (cw, ch) = (canvas.width().max(1) as u64, canvas.height().max(1) as u64);
        // Cheap exit: the mask region under the whole window is solid.
        let lo = |v: u32, d: u64, size: u32| ((v as u64 * size as u64) / d) as u32;
        let hi = |v: u32, d: u64, size: u32| (((v as u64 * size as u64).div_ceil(d)) as u32).min(size);
        let (rx0, rx1) = (lo(rect.0, dw, self.width), hi(rect.0 + rect.2, dw, self.width));
        let (ry0, ry1) = (lo(rect.1, dh, self.height), hi(rect.1 + rect.3, dh, self.height));
        if rx1 > rx0 && ry1 > ry0 && self.rect_sum(rx0, ry0, rx1, ry1) == (rx1 - rx0) * (ry1 - ry0) {
            return None;
        }
        let mut out: Option<image::RgbaImage> = None;
        for y in 0..canvas.height() {
            // Canvas pixel centre -> design y -> mask y.
            let dy = rect.1 as u64 * 2 * ch + (2 * y as u64 + 1) * rect.3 as u64;
            let my = ((dy * self.height as u64) / (2 * ch * dh)).min(self.height as u64 - 1) as u32;
            for x in 0..canvas.width() {
                let dx = rect.0 as u64 * 2 * cw + (2 * x as u64 + 1) * rect.2 as u64;
                let mx = ((dx * self.width as u64) / (2 * cw * dw)).min(self.width as u64 - 1) as u32;
                if self.rect_sum(mx, my, mx + 1, my + 1) == 0 {
                    out.get_or_insert_with(|| canvas.clone()).get_pixel_mut(x, y).0[3] = 0;
                }
            }
        }
        out
    }
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
        assert!(resolve_borderless(None, None, Some(&no_borders), None));
        assert!(resolve_borderless(None, Some(&GarglkOverlay::default()), Some(&no_borders), None));
        // garglk wborder wins over the cfg, in both directions.
        assert!(!resolve_borderless(None, Some(&garglk(false)), Some(&no_borders), None));
        let borders = GlkDesign { borders: Some(true), ..Default::default() };
        assert!(resolve_borderless(None, Some(&garglk(true)), Some(&borders), None));
        // the player's per-game choice wins over both.
        assert!(!resolve_borderless(Some(false), Some(&garglk(true)), Some(&no_borders), None));
        assert!(resolve_borderless(Some(true), None, None, None));
        // nothing says anything: bordered.
        assert!(!resolve_borderless(None, None, None, None));
    }

    #[test]
    fn borderless_user_default_sits_below_every_other_source() {
        let garglk = |b| GarglkOverlay { borderless: Some(b), ..GarglkOverlay::default() };
        let cfg = |b: bool| GlkDesign { borders: Some(!b), ..Default::default() };
        // Nothing else says anything: the user default applies, either way.
        assert!(resolve_borderless(None, None, None, Some(true)));
        assert!(!resolve_borderless(None, None, None, Some(false)));
        // A story .cfg design beats it.
        assert!(!resolve_borderless(None, None, Some(&cfg(false)), Some(true)));
        assert!(resolve_borderless(None, None, Some(&cfg(true)), Some(false)));
        // garglk.ini beats it (and the cfg).
        assert!(!resolve_borderless(None, Some(&garglk(false)), None, Some(true)));
        assert!(resolve_borderless(None, Some(&garglk(true)), Some(&cfg(false)), Some(false)));
        // The per-game override beats everything.
        assert!(!resolve_borderless(Some(false), Some(&garglk(true)), Some(&cfg(true)), Some(true)));
        assert!(resolve_borderless(Some(true), None, None, Some(false)));
        // No user default: bordered.
        assert!(!resolve_borderless(None, None, None, None));
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

    /// A `w x h` mask, opaque where `f(x, y)`.
    fn mask(w: u32, h: u32, f: impl Fn(u32, u32) -> bool) -> GlkMask {
        let px: Vec<bool> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
        GlkMask::from_opaque(w, h, &px).unwrap()
    }

    #[test]
    fn a_cell_under_half_inside_the_mask_is_hidden_exactly_at_the_threshold() {
        // 8x4 mask over a 2x1 grid: each cell is 4x4 = 16 mask pixels.
        // Cell 0 has `n` opaque pixels (filled row-major), cell 1 none.
        for n in 0..=16u32 {
            let m = mask(8, 4, |x, y| x < 4 && y * 4 + x < n);
            assert_eq!(m.cell_coverage(0, 0, 2, 1), (n, 16));
            assert_eq!(m.cell_visible(0, 0, 2, 1), n >= 8, "n = {n}: exactly half (8 of 16) stays");
            assert!(!m.cell_visible(1, 0, 2, 1), "all-out cell hidden");
        }
        let solid = mask(8, 4, |_, _| true);
        assert!(solid.visible_cells(2, 1).iter().all(|&v| v), "all-in");
    }

    #[test]
    fn coverage_stretches_per_axis_over_the_mask() {
        // Left 3/8 opaque, full height: x edge at 3 of 8 mask px; y is irrelevant.
        let m = mask(8, 6, |x, _| x < 3);
        // 4 cols x 3 rows: col edges 0,2,4,6,8 -> col 0 fully in, col 1 half
        // (x 2..4 has x=2 only), col 2/3 out.
        let v = m.visible_cells(4, 3);
        for row in 0..3usize {
            assert_eq!(&v[row * 4..row * 4 + 4], &[true, true, false, false], "row {row}");
        }
        // The same mask over a different grid: 5 cols, edges floor(i*8/5) = 0,1,3,4,6,8.
        assert_eq!(m.cell_coverage(1, 0, 5, 1), (2 * 6, 2 * 6), "x in 1..3 both opaque");
        assert_eq!(m.cell_coverage(2, 0, 5, 1), (0, 6), "x in 3..4 is outside");
        // Only the part inside counts: cols=5, col 2 spans x 3..4 -> 0 of 6.
        assert!(!m.cell_visible(2, 0, 5, 1));
        // Anisotropic: 2 cols x 6 rows.
        let top = mask(8, 6, |_, y| y < 2);
        let g = top.visible_cells(2, 6);
        assert_eq!(g.iter().filter(|&&v| v).count(), 2 * 2, "only the top 2 of 6 rows");
    }

    #[test]
    fn more_cells_than_mask_pixels_still_gives_each_cell_a_pixel() {
        let m = mask(2, 2, |x, y| x == 0 && y == 0);
        let v = m.visible_cells(8, 8);
        assert_eq!(v.iter().filter(|&&c| c).count(), 16, "the one opaque pixel owns a 4x4 block of cells");
    }

    #[test]
    fn white_or_clear_pixels_are_transparent_everything_else_opaque() {
        let mut img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(0, 1, image::Rgba([10, 20, 30, 0]));
        img.put_pixel(1, 1, image::Rgba([254, 255, 255, 255]));
        let m = GlkMask::from_rgba(&img).unwrap();
        assert_eq!(m.cell_coverage(0, 0, 2, 2).0, 1, "black opaque");
        assert_eq!(m.cell_coverage(1, 0, 2, 2).0, 0, "white transparent");
        assert_eq!(m.cell_coverage(0, 1, 2, 2).0, 0, "alpha 0 transparent");
        assert_eq!(m.cell_coverage(1, 1, 2, 2).0, 1, "near-white is opaque");
    }

    #[test]
    fn clip_canvas_zeroes_alpha_where_the_mask_is_transparent_in_design_space() {
        // 4x2 mask over a 40x20 design frame: left half opaque.
        let m = mask(4, 2, |x, _| x < 2);
        let canvas = image::RgbaImage::from_pixel(10, 20, image::Rgba([9, 9, 9, 255]));
        // A window at design (10, 0) 10x20: wholly in the opaque half -> untouched.
        assert!(m.clip_canvas(&canvas, (10, 0, 10, 20), (40, 20)).is_none());
        // A window at design (10, 0) 20x20 with a 20px canvas: its right half is outside.
        let wide = image::RgbaImage::from_pixel(20, 20, image::Rgba([9, 9, 9, 255]));
        let c = m.clip_canvas(&wide, (10, 0, 20, 20), (40, 20)).expect("partly outside");
        assert_eq!(c.get_pixel(9, 5).0[3], 255);
        assert_eq!(c.get_pixel(10, 5).0[3], 0);
        assert_eq!(c.get_pixel(19, 19).0[3], 0);
    }
}
