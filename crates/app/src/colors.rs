//! Color scheme support: Ghostty theme parsing and per-element color resolution.
//!
//! # Overview
//!
//! - [`GhosttyScheme`] holds the raw colors parsed from a Ghostty theme file.
//! - [`ColorScheme`] holds the resolved per-element colors used by the renderer.
//! - [`ColorScheme::terminal_default`] reproduces the hardcoded colors in the current renderer.
//! - [`ColorScheme::from_ghostty`] maps a parsed scheme onto UI elements.
//! - `resolve_base` is the live entry point: resolves a scheme name/path to a
//!   `(ColorScheme, GhosttyScheme, warnings)` triple used by `style::resolve`.

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

use crate::render::dialog::DialogPlacement;
use crate::render::paneframe::{BorderStyle, PaneGlyphs, PaneSides};

// ── ZMSD §8.3.1 standard colour table ────────────────────────────────────────

/// The Z-machine Standard colours 2..=9 paired with their ZMSD §8.3.1
/// recommended true-colour equivalents, as 15-bit `0bbbbbgggggrrrrr` values:
///
/// ```text
///  2 = black   (true $0000)    6 = blue    (true $59A0)
///  3 = red     (true $001D)    7 = magenta (true $7C1F)
///  4 = green   (true $0340)    8 = cyan    (true $77A0)
///  5 = yellow  (true $03BD)    9 = white   (true $7FFF)
/// ```
///
/// §8.3.1.1: "The equivalences between the colour numbers and true colours are
/// recommended. The interpreter may allow the user to change the mapping, but
/// the given values should be the default." This is that default, shared by the
/// pixel paths (`render::v6_layout::standard_pixel_rgb`) and the terminal cell
/// palette ([`ColorScheme::terminal_default`]) so one game colour looks the same
/// in both. Colours 10..=12 (v6-only greys) carry their own fixed RGB in
/// `zvm::screen::grey_rgb` and are not in this table.
pub const STANDARD_COLOUR_RGB15: [(u8, u16); 8] = [
    (2, 0x0000),
    (3, 0x001D),
    (4, 0x0340),
    (5, 0x03BD),
    (6, 0x59A0),
    (7, 0x7C1F),
    (8, 0x77A0),
    (9, 0x7FFF),
];

/// The true-colour equivalent of Standard colour `n` as 8-bit RGB, or `None`
/// outside 2..=9.
///
/// Resolved through `zvm::screen::standard_true_colour`, so the active
/// interpreter palette applies (SQ-0719): under the default
/// `Palette::Standard` this is exactly [`STANDARD_COLOUR_RGB15`] above; under
/// `Palette::Amiga` these become the colours Infocom's own Amiga interpreter
/// loaded. Routing through the VM's table rather than reading the local one
/// keeps a single answer for "what colour is Standard(6)" across the terminal
/// cell palette, the v6 pixel path and the game's own window properties 17/18.
pub fn standard_colour_rgb(n: u8) -> Option<(u8, u8, u8)> {
    if !STANDARD_COLOUR_RGB15.iter().any(|(c, _)| *c == n) {
        return None;
    }
    zvm::screen::standard_true_colour(n).map(zvm::screen::rgb15_to_888)
}

/// The §8.3.1 true-colour equivalent of Standard colour `n` as a concrete
/// `Color::Rgb`. Panics outside 2..=9 — the only callers are the fixed 2..=9
/// palette seed below.
fn standard_colour(n: u8) -> Color {
    let (r, g, b) = standard_colour_rgb(n).expect("standard colour 2..=9");
    Color::Rgb(r, g, b)
}

/// The Standard colour (2..=9) whose §8.3.1 true-colour equivalent sits nearest
/// `rgb`, by squared Euclidean distance in RGB space. Ties go to the LOWER
/// colour number (so a colour exactly between black and white reads as black),
/// which keeps the mapping deterministic.
///
/// Used to express the app's own default page/ink as the standard codes the
/// header can carry (§8.3.3 — bytes $2C/$2D hold colour NUMBERS, not RGB).
pub fn nearest_standard_colour(rgb: (u8, u8, u8)) -> u8 {
    let (r, g, b) = (rgb.0 as i32, rgb.1 as i32, rgb.2 as i32);
    let mut best = (2u8, i32::MAX);
    for (n, _) in STANDARD_COLOUR_RGB15 {
        let (cr, cg, cb) = standard_colour_rgb(n).expect("table entry");
        let d = (r - cr as i32).pow(2) + (g - cg as i32).pow(2) + (b - cb as i32).pow(2);
        if d < best.1 {
            best = (n, d);
        }
    }
    best.0
}

/// Resolve the interpreter's OWN default page + ink and express it as a ZMSD
/// §8.3.1 standard colour pair, returned as `(background, foreground)` — the
/// order of header bytes $2C/$2D.
///
/// §8.3.3: "If the interpreter can produce colours, it should set bit 0 of
/// 'Flags 1' in the header, and write its default background and foreground
/// colours into bytes $2c and $2d of the header." Those bytes should therefore
/// describe what the player actually sees, not a fixed black/white guess.
///
/// The pair is drawn from ONE layer, exactly like the v6 raster canvas's
/// `v6_default_pair` (SQ-0510): the theme, but only when it supplies BOTH
/// channels as concrete RGB; else the OSC 10/11-probed terminal pair, but only
/// when the terminal answered BOTH. A half-resolved layer is skipped whole
/// rather than mixed, so page and ink always come from the same design. When
/// neither layer resolves, `None` — the caller leaves the interpreter's own
/// black-on-white seed (§8.3.2's colours 2 and 9) in place.
pub fn host_default_colour_pair(
    themed: Style,
    osc_fg: Option<(u8, u8, u8)>,
    osc_bg: Option<(u8, u8, u8)>,
) -> Option<(u8, u8)> {
    fn rgb(c: Option<Color>) -> Option<(u8, u8, u8)> {
        match c {
            Some(Color::Rgb(r, g, b)) => Some((r, g, b)),
            _ => None,
        }
    }
    let (fg, bg) = match (rgb(themed.fg), rgb(themed.bg)) {
        (Some(f), Some(b)) => (f, b),
        _ => match (osc_fg, osc_bg) {
            (Some(f), Some(b)) => (f, b),
            _ => return None,
        },
    };
    Some((nearest_standard_colour(bg), nearest_standard_colour(fg)))
}

/// The page and ink this launch reports to the story in header `$2C`/`$2D`,
/// from the source it licenses (SQ-1082).
///
/// The three arms are what `--colour terminal|theme|machine` names, and they are
/// not new: this is the `or_else` chain `startup.rs` and `reset.rs` each kept
/// their own copy of, with a switch on which arm it may start at. `machine` is
/// the default and is the whole chain, so a launch that says nothing resolves
/// exactly as it did before.
///
/// `machine` is passed in rather than read off `cfg`, because the caller has
/// already folded in the two-colour CARD's pair where the archive supplies one
/// (SQ-0956) — a fact about the same machine's display, and the strongest arm of
/// the same source.
///
/// **`honor_game_colours = false` answers no** — and that coupling is deliberate,
/// not the coincidence of one branch it used to be. `--game-colours off` declares
/// the interpreter colourless under ZMSD §8.3.2, and an interpreter with no
/// colours has no default page and ink to report: the VM's own black-on-white
/// seed is what §8.3.2 asks be left alone. So the two flags are different axes —
/// one is whether the story's requests are obeyed, the other is what DEFAULT
/// resolves to — and `--colour` is simply inert while the first says off. The
/// theme still paints the pane either way; that is a fact about lanthorn's
/// window, not a claim made to the story.
pub fn host_default_colours(
    cfg: &crate::config::Config,
    machine: Option<(u8, u8)>,
    themed: Style,
    osc_fg: Option<(u8, u8, u8)>,
    osc_bg: Option<(u8, u8, u8)>,
) -> Option<(u8, u8)> {
    use crate::config::ColourSource;
    if !cfg.honor_game_colours {
        return None;
    }
    match cfg.colour_source {
        ColourSource::Machine => {
            machine.or_else(|| host_default_colour_pair(themed, osc_fg, osc_bg))
        }
        ColourSource::Theme => host_default_colour_pair(themed, osc_fg, osc_bg),
        // The theme is dropped by handing the pair resolver a style that names
        // neither channel, which is the state it already falls through on.
        ColourSource::Terminal => host_default_colour_pair(Style::default(), osc_fg, osc_bg),
    }
}

/// Seed an unconfigured base scheme's `foreground`/`background` from the
/// terminal's OSC 10/11-probed defaults (SQ-0510).
///
/// With no `scheme` configured, [`resolve_base`] hands back a
/// [`GhosttyScheme::default()`] whose fg/bg/palette are all [`Color::Reset`], and
/// [`Roles::from_scheme`](crate::theme::resolve::Roles::from_scheme) early-returns
/// on exactly that to the concrete
/// [`Roles::terminal_default`](crate::theme::resolve::Roles::terminal_default)
/// palette — whose `chrome` role is a hard-coded white-on-BLACK. That guess is
/// only right on a black-background terminal; anywhere else the chrome-derived
/// selectors (`status_bar`, `upper_window`, `story_info`, `dialog.background`,
/// the `glk.grid.*` family) paint a black band across a terminal that is not
/// black. The probe already knows the real answer — this is what hands it to the
/// theme.
///
/// Applies ONLY when the scheme is the unconfigured all-`Reset` one: a real
/// configured scheme (built-in name or file) is the user's explicit choice and
/// is never second-guessed. The palette is left untouched, so `from_scheme`'s
/// per-slot fallback still supplies the cyan/yellow/grey accents.
///
/// A PARTIAL probe answer (fg without bg, or bg without fg) is skipped whole
/// rather than mixed, matching the house rule [`host_default_colour_pair`]
/// documents: page and ink always come from the same design, never a probed ink
/// on a guessed page. A terminal that answers neither leaves the scheme
/// untouched and keeps the `terminal_default()` behaviour byte-for-byte.
pub fn seed_scheme_from_terminal(
    mut gs: GhosttyScheme,
    term: &crate::term_colors::TermDefaultColors,
) -> GhosttyScheme {
    if gs.foreground != Color::Reset || gs.background != Color::Reset {
        return gs; // a configured scheme speaks for itself
    }
    let (Some(fg), Some(bg)) = (term.fg, term.bg) else {
        return gs; // no answer, or a half answer — skip the layer whole
    };
    gs.foreground = Color::Rgb(fg.0[0], fg.0[1], fg.0[2]);
    gs.background = Color::Rgb(bg.0[0], bg.0[1], bg.0[2]);
    gs
}

/// A compiled user transcript-styling rule: a regex matched whole-line against
/// Story text, plus the `Style` patched over the base `transcript` style on a
/// match. `PartialEq` compares the source `pattern` and `style` only — the
/// compiled `Regex` has no `PartialEq`, and two rules with the same pattern are
/// equal by construction.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub pattern: String,
    pub regex: Regex,
    pub style: Style,
}

impl PartialEq for CompiledRule {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.style == other.style
    }
}

/// Which alignment cluster a status-bar segment belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    /// The lowercase config name for this alignment.
    pub fn as_str(&self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

/// One resolved status-bar segment: a text template, its cluster, and the style
/// patched over the base `statusbar` style.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSegment {
    pub text: String,
    pub align: Align,
    pub style: Style,
}

/// The ordered list of status-bar segments. `Default` is the built-in layout that
/// reproduces today's bar (location left; score/moves or clock right; filter right).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBarLayout {
    pub segments: Vec<StatusSegment>,
}

impl Default for StatusBarLayout {
    fn default() -> Self {
        let seg = |text: &str, align: Align| StatusSegment {
            text: text.to_string(),
            align,
            style: Style::default(),
        };
        StatusBarLayout {
            segments: vec![
                seg("{location}", Align::Left),
                seg("Score: {score}  Moves: {moves}", Align::Right),
                seg("{time}", Align::Right),
                seg(" {filter}", Align::Right),
            ],
        }
    }
}

// ── Built-in theme texts ──────────────────────────────────────────────────────

const BUILTIN_MONO: &str = include_str!("colors/mono.ghostty");
const BUILTIN_HIGH_CONTRAST: &str = include_str!("colors/high-contrast.ghostty");
const BUILTIN_TOMORROW_NIGHT: &str = include_str!("colors/tomorrow-night.ghostty");

// ── GhosttyScheme ─────────────────────────────────────────────────────────────

/// The raw colors loaded from a Ghostty theme file.
///
/// Ghostty theme files use `key = value` syntax (one per line).  Relevant keys:
/// `palette = N=#rrggbb` (or `rrggbb`), `background`, `foreground`,
/// `cursor-color`, `selection-background`, `selection-foreground`.
/// All other keys are silently ignored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GhosttyScheme {
    /// The 16-color ANSI palette.  Entries that were not specified in the file
    /// default to `Color::Reset`.
    pub palette: [Color; 16],
    pub background: Color,
    pub foreground: Color,
    pub cursor: Option<Color>,
    pub selection_bg: Option<Color>,
    pub selection_fg: Option<Color>,
}

impl GhosttyScheme {
    /// Parse a Ghostty theme file text.  Returns `Err` only when `background`
    /// or `foreground` is missing from the file (they are required for a
    /// meaningful scheme).  All other parsing errors on individual lines are
    /// silently skipped.
    pub fn parse(text: &str) -> Result<GhosttyScheme, String> {
        let mut palette: [Color; 16] = [Color::Reset; 16];
        let mut background: Option<Color> = None;
        let mut foreground: Option<Color> = None;
        let mut cursor: Option<Color> = None;
        let mut selection_bg: Option<Color> = None;
        let mut selection_fg: Option<Color> = None;

        for line in text.lines() {
            // Strip comments and whitespace.
            let line = match line.find('#') {
                // '#' is only a comment when it is at the start of the value
                // OR the whole line.  In Ghostty palette lines the '#' is part
                // of the hex color, so only strip a leading standalone '#'.
                Some(_) if line.trim_start().starts_with('#') => continue,
                _ => line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "palette" => {
                    // Format: N=#rrggbb or N=rrggbb
                    if let Some((idx_s, hex_s)) = value.split_once('=') {
                        let idx_s = idx_s.trim();
                        let hex_s = hex_s.trim();
                        if let Ok(idx) = idx_s.parse::<usize>() {
                            if idx < 16 {
                                if let Some(c) = parse_hex_color(hex_s) {
                                    palette[idx] = c;
                                }
                            }
                        }
                    }
                }
                "background" => {
                    if let Some(c) = parse_hex_color(value) {
                        background = Some(c);
                    }
                }
                "foreground" => {
                    if let Some(c) = parse_hex_color(value) {
                        foreground = Some(c);
                    }
                }
                "cursor-color" => {
                    cursor = parse_hex_color(value);
                }
                "selection-background" => {
                    selection_bg = parse_hex_color(value);
                }
                "selection-foreground" => {
                    selection_fg = parse_hex_color(value);
                }
                _ => {} // ignore unknown keys
            }
        }

        let background = background.ok_or_else(|| "missing 'background' key".to_string())?;
        let foreground = foreground.ok_or_else(|| "missing 'foreground' key".to_string())?;

        Ok(GhosttyScheme {
            palette,
            background,
            foreground,
            cursor,
            selection_bg,
            selection_fg,
        })
    }
}

// ── ColorScheme ───────────────────────────────────────────────────────────────

/// Per-element resolved colors for the renderer.
///
/// Each field is a [`Style`] ready to apply with ratatui.  The renderer should
/// use these instead of its hardcoded color constants once the renderer-wiring
/// track connects `AppState.colors` to the render functions.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    /// Transcript text (body of transcript pane). SQ-0309: kept — unlike every
    /// other single-`Style` colour field (all deleted; render now reads
    /// `theme.get("<selector>").style`), this one still has a live production
    /// reader with a real ordering hazard: `glk_backend::theme_style_colours`
    /// is called in `startup.rs` to seed the engine's boot-time
    /// `glk_style_measure` answer (SQ-0803: the per-Glk-style slots resolved over
    /// this base — Kerkerkruip probes its style_User2 slot there)
    /// BEFORE `state.colors.theme` is rebuilt with the layered
    /// global/garglk/per-game decls (that rebuild happens later, in
    /// `reload_style`). A discovered `garglk.ini` overlay patches this field
    /// directly (`GarglkOverlay::apply`) so the boot-time snapshot reflects it;
    /// migrating to `theme.get("transcript")` at that call site would silently
    /// lose the garglk-overlaid colour for the first-frame boot probe. Every
    /// other read of `transcript` (e.g. `resolve_story_style`'s base) is safe
    /// and stays on this field too, rather than mixing sources.
    pub transcript: Style,
    /// Resolved border style for the map pane.
    pub map_border_style: BorderStyle,
    /// Resolved border style for the story pane.
    pub story_border_style: BorderStyle,
    /// Resolved border style for the status header.
    pub status_header_style: BorderStyle,
    /// Resolved border style for the input line.
    pub input_line_style: BorderStyle,
    /// Resolved border style for the auto-complete suggestion popup.
    pub suggestion_line_style: BorderStyle,
    /// Border style for notification toasts (default `Single`). (SQ-0176)
    pub notification_style: BorderStyle,
    /// Resolved border style for the dialog box.
    pub dialog_box_style: BorderStyle,
    /// Whether the dialog drop-shadow is enabled.
    pub dialog_shadow_on: bool,
    /// Where centered modals are anchored on screen (default `Center`).
    pub dialog_placement: DialogPlacement,
    /// Cells of gap from the anchored edge(s); ignored for `Center` (default `0`).
    pub dialog_margin: u16,
    /// Resolved border style for the upper (virtual) window frame.
    pub virtual_window_border: BorderStyle,
    /// Per-side border styles (default = all of the matching base `*_style`).
    pub map_border_sides: PaneSides,
    pub story_border_sides: PaneSides,
    pub status_header_sides: PaneSides,
    pub input_line_sides: PaneSides,
    pub suggestion_line_sides: PaneSides,
    pub notification_sides: PaneSides,
    pub upper_window_border_sides: PaneSides,
    /// Per-side/corner glyph overrides for each bordered pane element.
    pub map_border_glyphs: PaneGlyphs,
    pub story_border_glyphs: PaneGlyphs,
    pub status_header_glyphs: PaneGlyphs,
    pub input_line_glyphs: PaneGlyphs,
    pub suggestion_line_glyphs: PaneGlyphs,
    pub notification_glyphs: PaneGlyphs,
    pub upper_window_border_glyphs: PaneGlyphs,
    pub dialog_glyphs: PaneGlyphs,
    /// Whether the story title / map layer-tab header strip is shown.
    pub story_header_on: bool,
    pub map_header_on: bool,
    /// Compiled user story-styling rules, in evaluation order.
    pub transcript_rules: Vec<CompiledRule>,
    /// The status-bar segment layout (default reproduces today's bar).
    pub statusbar_layout: StatusBarLayout,
    /// The 16-colour ANSI palette carried from the terminal theme (indices 0-15).
    /// Used by `resolve_zcolour` to map `ZColour::Standard(2..=9)` through the
    /// user's colour scheme rather than the Z-machine's raw ANSI numbers.
    pub palette: [Color; 16],
    /// Per-Glk-style theme colours, resolved between the game's stylehint and the
    /// per-app-element base (SQ-0331). Row 0 = text-buffer windows (base
    /// `transcript`); row 1 = text-grid windows (base `upper_window`). Indexed by
    /// the Glk style class (0=Normal .. 10=User2). A slot's `None` channel falls
    /// through to the element, so a `Default` (all-`None`) array leaves the
    /// Z-machine (all-Normal) render byte-identical. Seeded here (buffer Input←
    /// `input_text`, buffer Subheader←`transcript_location`); the SQ-0319
    /// garglk.ini importer will populate the rest. Not yet editable via style.toml
    /// (deferred to the SQ-0309 style redesign).
    pub glk_styles: [[Style; 11]; 2],
    /// SQ-0309: the registry-resolved theme, carried alongside the legacy fields
    /// during the migration. Later waves read `theme.get("<selector>")` instead of
    /// the individual fields above; this is populated from the same scheme/roles as
    /// those fields and must reproduce them.
    pub theme: crate::theme::resolve::Theme,
}

/// Build the seed `glk_styles` array (SQ-0331): buffer Input(8) ← `input_text`,
/// buffer Subheader(4) ← `transcript_location`; every other slot inherits its
/// element (left `None`), so Normal is definitionally the element and the
/// Z-machine render stays byte-identical.
fn seed_glk_styles(input_text: Style, transcript_location: Style) -> [[Style; 11]; 2] {
    let mut styles = [[Style::default(); 11]; 2];
    styles[0][8] = Style { fg: input_text.fg, bg: input_text.bg, ..Style::default() };
    styles[0][4] = Style { fg: transcript_location.fg, bg: transcript_location.bg, ..Style::default() };
    styles
}

impl ColorScheme {
    /// Reproduce the exact colors hardcoded in today's renderer constants.
    ///
    /// Matches:
    /// - `render/map.rs`: `CURRENT_STYLE`, `SELECTED_STYLE`, `NORMAL_STYLE`, `CONNECTOR_STYLE`,
    ///   plus the inline `Cyan`/`Magenta` colors and the portal-connector `Cyan`.
    /// - `render/transcript.rs`: `STATUS_STYLE`, `NORMAL_STYLE`, and the `DarkGray` suggestion.
    /// - `main.rs`: `focused_border` (`Cyan + BOLD`) and `help_style` (`REVERSED`).
    pub fn terminal_default() -> ColorScheme {
        ColorScheme {
            transcript: Style::new().fg(Color::White),
            // SQ-0309: previously injected by DEFAULT_STYLE_TOML's `[colors]`
            // "map_border"/"story_border" `style = "single"` (applied via the
            // now-removed `apply_color_decls`). That was the only path that ever
            // set these away from `None` for the built-in default look, so the
            // literal default moves here to keep it (render/map.rs's layer-strip
            // suppression and the single-line pane border still depend on it).
            map_border_style: BorderStyle::Single,
            story_border_style: BorderStyle::Single,
            status_header_style: BorderStyle::None,
            input_line_style: BorderStyle::None,
            suggestion_line_style: BorderStyle::None,
            notification_style: BorderStyle::Single,
            dialog_box_style: BorderStyle::None,
            dialog_shadow_on: false,
            dialog_placement: DialogPlacement::Center,
            dialog_margin: 0,
            virtual_window_border: BorderStyle::None,
            map_border_sides: PaneSides::all(BorderStyle::Single),
            story_border_sides: PaneSides::all(BorderStyle::Single),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            suggestion_line_sides: PaneSides::all(BorderStyle::None),
            notification_sides: PaneSides::all(BorderStyle::Single),
            upper_window_border_sides: PaneSides::all(BorderStyle::None),
            map_border_glyphs: PaneGlyphs::default(),
            story_border_glyphs: PaneGlyphs::default(),
            status_header_glyphs: PaneGlyphs::default(),
            input_line_glyphs: PaneGlyphs::default(),
            suggestion_line_glyphs: PaneGlyphs::default(),
            notification_glyphs: PaneGlyphs::default(),
            upper_window_border_glyphs: PaneGlyphs::default(),
            dialog_glyphs: PaneGlyphs::default(),
            story_header_on: true,
            map_header_on: true,
            transcript_rules: Vec::new(),
            statusbar_layout: StatusBarLayout::default(),
            palette: [
                // Slots 0..=7 are the Z-machine Standard colours 2..=9 (see
                // `resolve_zcolour`). ZMSD §8.3.1.1: "The equivalences between the
                // colour numbers and true colours are recommended. The interpreter
                // may allow the user to change the mapping, but the given values
                // should be the default." So the BUILT-IN default seeds them from
                // the §8.3.1 true-colour table as concrete RGB rather than named
                // ANSI colours — the named mapping sent Standard white(9) to
                // `Color::Gray` (the dim VGA base white), which disagreed with the
                // exact RGB the v6 raster path already used (`standard_pixel_rgb`),
                // so the same game colour looked different in cell and pixel paths.
                // A user theme still overrides the whole palette (SQ-0532/A-F5).
                standard_colour(2),  // 0  Z Standard(2) black
                standard_colour(3),  // 1  Standard(3) red
                standard_colour(4),  // 2  Standard(4) green
                standard_colour(5),  // 3  Standard(5) yellow
                standard_colour(6),  // 4  Standard(6) blue
                standard_colour(7),  // 5  Standard(7) magenta
                standard_colour(8),  // 6  Standard(8) cyan
                standard_colour(9),  // 7  Standard(9) white
                Color::DarkGray,   // 8  bright black
                Color::LightRed,   // 9
                Color::LightGreen, // 10
                Color::LightYellow,// 11
                Color::LightBlue,  // 12
                Color::LightMagenta,//13
                Color::LightCyan,  // 14
                Color::White,      // 15 bright white
            ],
            glk_styles: seed_glk_styles(Style::new(), Style::new().add_modifier(Modifier::BOLD)),
            theme: crate::theme::resolve::resolve(
                &crate::theme::resolve::Roles::terminal_default(),
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
            ),
        }
    }

    /// Build a `ColorScheme` from a parsed `GhosttyScheme` and optional per-element overrides.
    ///
    /// # Default element→role mapping
    ///
    /// | Element             | Ghostty role          |
    /// |---------------------|-----------------------|
    /// | `transcript`        | `foreground`          |
    ///
    /// Overrides in `elements` map element names to color values (parsed by
    /// [`parse_color_value`]) and beat the default mapping.
    pub fn from_ghostty(
        scheme: &GhosttyScheme,
        overrides: &BTreeMap<String, String>,
    ) -> ColorScheme {
        let fg = scheme.foreground;

        // Helper: look up element override or fall back to the default color.
        let resolve_element = |name: &str, default: Color| -> Color {
            overrides
                .get(name)
                .and_then(|v| parse_color_value(v, scheme))
                .unwrap_or(default)
        };

        let transcript_fg = resolve_element("transcript", fg);

        ColorScheme {
            transcript: Style::new().fg(transcript_fg),
            // SQ-0641: a colour scheme recolours, it must not change border
            // STRUCTURE. These used to be seeded `None` here and then flipped to
            // Single by the (since-removed) `apply_color_decls` pass over
            // DEFAULT_STYLE_TOML's map_border/story_border decls; when that pass
            // went away only `terminal_default()` was fixed up, so configuring
            // any scheme silently dropped the pane borders (and re-enabled the
            // in-content map layer strip → double tab strip). Match
            // `terminal_default()`.
            map_border_style: BorderStyle::Single,
            story_border_style: BorderStyle::Single,
            status_header_style: BorderStyle::None,
            input_line_style: BorderStyle::None,
            suggestion_line_style: BorderStyle::None,
            notification_style: BorderStyle::Single,
            dialog_box_style: BorderStyle::None,
            dialog_shadow_on: false,
            dialog_placement: DialogPlacement::Center,
            dialog_margin: 0,
            virtual_window_border: BorderStyle::None,
            map_border_sides: PaneSides::all(BorderStyle::Single),
            story_border_sides: PaneSides::all(BorderStyle::Single),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            suggestion_line_sides: PaneSides::all(BorderStyle::None),
            notification_sides: PaneSides::all(BorderStyle::Single),
            upper_window_border_sides: PaneSides::all(BorderStyle::None),
            map_border_glyphs: PaneGlyphs::default(),
            story_border_glyphs: PaneGlyphs::default(),
            status_header_glyphs: PaneGlyphs::default(),
            input_line_glyphs: PaneGlyphs::default(),
            suggestion_line_glyphs: PaneGlyphs::default(),
            notification_glyphs: PaneGlyphs::default(),
            upper_window_border_glyphs: PaneGlyphs::default(),
            dialog_glyphs: PaneGlyphs::default(),
            story_header_on: true,
            map_header_on: true,
            transcript_rules: Vec::new(),
            statusbar_layout: StatusBarLayout::default(),
            palette: scheme.palette,
            glk_styles: seed_glk_styles(Style::new(), Style::new().add_modifier(Modifier::BOLD)),
            theme: crate::theme::resolve::resolve_theme(scheme, &crate::theme::toml_schema::ParsedStyle::default()),
        }
    }

    /// Resolve the style for one Story line: first matching user rule wins, else
    /// the built-in location rule (line matches `room_name`), else the built-in
    /// system rule (whole line bracketed), else `base` unpatched. A match patches
    /// its style over `base` (overriding only set fields).
    ///
    /// `base` is normally this scheme's own `transcript` style; under ZMSD §8.3's
    /// Amiga interpreter it is that style with the MACHINE's ink and page laid
    /// under it (`render::screen::v6_machine_page`), because there an inherited
    /// channel belongs to the machine and not to the theme.
    ///
    /// `machine_owns_ink` is true on exactly those frames, and it withdraws the
    /// built-in **system** rule (SQ-0822). That rule is a guess — "a whole line in
    /// brackets came from the interpreter, not from the game" — and its payload is
    /// a MUTED colour, chosen to recede against the theme's own page. Both halves
    /// fail here: Arthur release 54 prints `[You have earned ten chivalry points.]`
    /// as ordinary prose in the machine's own pens, and muting it against the
    /// Amiga's dark-grey page leaves dark grey on dark grey, which is what the
    /// symptom looked like. On a machine with one pair for the whole screen there
    /// is no third colour to recede into.
    ///
    /// The other two rules still fire. A user `transcript_rules` entry is explicit
    /// configuration and must always win. The built-in LOCATION rule survives
    /// because it differs in kind: it is a reading aid, it paints an accent rather
    /// than a mute, and an accent is legible on any page — whereas the system rule
    /// asserts something about the line's provenance that is simply untrue here.
    pub fn resolve_story_style(
        &self,
        base: Style,
        line: &str,
        room_name: Option<&str>,
        machine_owns_ink: bool,
    ) -> Style {
        for rule in &self.transcript_rules {
            if rule.regex.is_match(line) {
                return base.patch(rule.style);
            }
        }
        if let Some(name) = room_name {
            if zvm::location::status_name_matches(line, name) {
                return base.patch(self.theme.get("transcript_location").style);
            }
        }
        let t = line.trim();
        if !machine_owns_ink && t.len() >= 2 && t.starts_with('[') && t.ends_with(']') {
            return base.patch(self.theme.get("transcript_system").style);
        }
        base
    }

}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::terminal_default()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolve a scheme name/path to a `(ColorScheme, GhosttyScheme, warnings)` triple.
///
/// - `scheme == None` → returns `(terminal_default(), GhosttyScheme::default(), [])`
/// - A known built-in name or a file path → parses the Ghostty theme and returns
///   `(ColorScheme::from_ghostty(&gs, &empty), gs, [])`.
/// - Parse/read failure → returns `(terminal_default(), GhosttyScheme::default(), [warning])`.
///
/// The caller is responsible for applying element overrides on top of the returned
/// `ColorScheme` if needed.
pub(crate) fn resolve_base(
    scheme: Option<&str>,
    dir: &Path,
) -> (ColorScheme, GhosttyScheme, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();

    let name = match scheme {
        None => return (ColorScheme::terminal_default(), GhosttyScheme::default(), warnings),
        Some(n) => n,
    };

    let gs = match builtin_scheme_text(name) {
        Some(text) => match GhosttyScheme::parse(text) {
            Ok(gs) => gs,
            Err(e) => {
                warnings.push(format!(
                    "built-in scheme '{}' failed to parse: {}; using terminal defaults",
                    name, e
                ));
                return (ColorScheme::terminal_default(), GhosttyScheme::default(), warnings);
            }
        },
        None => {
            let path = expand_path(name, dir);
            match std::fs::read_to_string(&path) {
                Ok(text) => match GhosttyScheme::parse(&text) {
                    Ok(gs) => gs,
                    Err(e) => {
                        warnings.push(format!(
                            "scheme file '{}' failed to parse: {}; using terminal defaults",
                            path.display(),
                            e
                        ));
                        return (
                            ColorScheme::terminal_default(),
                            GhosttyScheme::default(),
                            warnings,
                        );
                    }
                },
                Err(e) => {
                    warnings.push(format!(
                        "could not read scheme file '{}': {}; using terminal defaults",
                        path.display(),
                        e
                    ));
                    return (
                        ColorScheme::terminal_default(),
                        GhosttyScheme::default(),
                        warnings,
                    );
                }
            }
        }
    };

    let empty_overrides = std::collections::BTreeMap::new();
    let cs = ColorScheme::from_ghostty(&gs, &empty_overrides);
    (cs, gs, warnings)
}

/// Return the embedded Ghostty theme text for a known built-in name, or `None`.
fn builtin_scheme_text(name: &str) -> Option<&'static str> {
    match name {
        "mono" => Some(BUILTIN_MONO),
        "high-contrast" => Some(BUILTIN_HIGH_CONTRAST),
        "tomorrow-night" => Some(BUILTIN_TOMORROW_NIGHT),
        _ => None,
    }
}

/// Expand `~` in a path string and resolve relative paths against `base_dir`.
pub fn expand_path(s: &str, base_dir: &Path) -> std::path::PathBuf {
    let expanded = if s.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(s.trim_start_matches("~/").trim_start_matches('~'))
    } else {
        std::path::PathBuf::from(s)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        base_dir.join(expanded)
    }
}

/// Parse a hex color string (`#rrggbb` or `rrggbb`) into `Color::Rgb`.
/// Returns `None` on invalid input.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

/// Parse a color value from a `[colors.elements]` entry.
///
/// Accepted formats:
/// - `palette:N`  — index 0-15 into the scheme's palette (requires a scheme)
/// - `background` / `foreground` — the scheme's bg/fg (requires a scheme)
/// - A named ratatui color (`cyan`, `yellow`, …) — case-insensitive
/// - A decimal 256-index (`"17"`)
/// - A hex color (`#5fafd7` or `5fafd7`)
///
/// Returns `None` if the value cannot be parsed.
pub fn parse_color_value(value: &str, scheme: &GhosttyScheme) -> Option<Color> {
    let v = value.trim();

    // palette:N
    if let Some(rest) = v.strip_prefix("palette:") {
        if let Ok(idx) = rest.trim().parse::<usize>() {
            if idx < 16 {
                return Some(scheme.palette[idx]);
            }
        }
        return None;
    }

    // scheme-relative roles
    match v {
        "background" => return Some(scheme.background),
        "foreground" => return Some(scheme.foreground),
        "default" => return Some(Color::Reset),
        // Explicit "unset" sentinel for a colour that is None. Resolves to no
        // colour (patches nothing) so an unset field stays unset — distinct from
        // "default"/"reset" (explicit terminal default).
        "none" => return None,
        _ => {}
    }

    // ratatui named colors (case-insensitive)
    if let Some(c) = parse_named_color(v) {
        return Some(c);
    }

    // 256-index: at most 3 decimal digits ("17", "255"). SQ-0642: a longer
    // all-digit string is NOT an index — a hashless 6-digit hex colour whose
    // digits happen to read ≤255 after leading zeros ("000080") used to win the
    // u8 parse here and come out as Indexed(80) instead of Rgb(0,0,128).
    if v.len() <= 3 && v.bytes().all(|b| b.is_ascii_digit()) {
        return v.parse::<u8>().ok().map(Color::Indexed);
    }

    // hex (documented: #rrggbb or rrggbb — a 6-char all-digit string lands here)
    parse_hex_color(v)
}

/// Parse a ratatui named color (case-insensitive).
///
/// Accepts the standard ANSI names (`black`, `red`, … `white`) and their
/// `bright-*` / `light-*` / `dark-*` variants. `bright-black` and `dark-black`
/// both map to `DarkGray`.
pub fn parse_named_color(s: &str) -> Option<Color> {
    match s.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "white" => Some(Color::White),
        "reset" => Some(Color::Reset),
        // dark- variants
        "dark-gray" | "dark-grey" | "darkgray" | "dark_gray" | "darkgrey" | "dark_grey"
        | "bright-black" | "dark-black" => Some(Color::DarkGray),
        // light- / bright- variants
        "light-red" | "lightred" | "light_red" | "bright-red" => Some(Color::LightRed),
        "light-green" | "lightgreen" | "light_green" | "bright-green" => Some(Color::LightGreen),
        "light-yellow" | "lightyellow" | "light_yellow" | "bright-yellow" => {
            Some(Color::LightYellow)
        }
        "light-blue" | "lightblue" | "light_blue" | "bright-blue" => Some(Color::LightBlue),
        "light-magenta" | "lightmagenta" | "light_magenta" | "bright-magenta" => {
            Some(Color::LightMagenta)
        }
        "light-cyan" | "lightcyan" | "light_cyan" | "bright-cyan" => Some(Color::LightCyan),
        "light-white" | "bright-white" => Some(Color::White),
        "light-black" | "bright-gray" | "bright-grey" | "light-gray" | "light-grey" => {
            Some(Color::Gray)
        }
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;

    #[test]
    fn seed_glk_styles_copies_input_and_subheader_leaves_rest_none() {
        let styles = seed_glk_styles(
            Style::new().fg(Color::Cyan).bg(Color::Black),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        );
        // Buffer (row 0): Input(8) ← input_text fg/bg; Subheader(4) ← transcript_location.
        assert_eq!(
            styles[0][8],
            Style { fg: Some(Color::Cyan), bg: Some(Color::Black), ..Style::default() }
        );
        assert_eq!(styles[0][4], Style { fg: Some(Color::Green), ..Style::default() });
        // Normal(0) is always None → definitionally the element (byte-identical Z-machine).
        assert_eq!(styles[0][0], Style::default());
        // Every other buffer slot and the entire grid row inherit (None).
        for i in [1usize, 2, 3, 5, 6, 7, 9, 10] {
            assert_eq!(styles[0][i], Style::default(), "buffer slot {i} inherits");
        }
        for (i, slot) in styles[1].iter().enumerate() {
            assert_eq!(*slot, Style::default(), "grid slot {i} inherits (row 1 unseeded)");
        }
    }

    #[test]
    fn terminal_default_glk_styles_are_all_none() {
        // input_text / transcript_location carry no concrete colour in the terminal
        // default, so the seeds resolve to None and Normal stays the element.
        let cs = ColorScheme::terminal_default();
        for row in 0..2 {
            for i in 0..11 {
                assert_eq!(cs.glk_styles[row][i], Style::default(), "row {row} slot {i}");
            }
        }
    }

    #[test]
    fn border_sides_default_to_all_of_base_and_headers_on() {
        use crate::render::paneframe::{PaneSides, BorderStyle};
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.map_border_sides, PaneSides::all(cs.map_border_style));
        assert_eq!(cs.story_border_sides, PaneSides::all(cs.story_border_style));
        assert_eq!(cs.status_header_sides, PaneSides::all(cs.status_header_style));
        assert_eq!(cs.input_line_sides, PaneSides::all(cs.input_line_style));
        assert_eq!(cs.suggestion_line_sides, PaneSides::all(cs.suggestion_line_style));
        assert_eq!(cs.notification_sides, PaneSides::all(cs.notification_style));
        assert_eq!(cs.notification_style, BorderStyle::Single, "toasts default to a single border");
        assert_eq!(cs.upper_window_border_sides, PaneSides::all(cs.virtual_window_border));
        assert!(cs.story_header_on);
        assert!(cs.map_header_on);
        // Single base (SQ-0309: the default single-line map/story border, formerly
        // injected by DEFAULT_STYLE_TOML) → all sides Single.
        assert_eq!(cs.map_border_sides, PaneSides::all(BorderStyle::Single));
    }

    #[test]
    fn statusbar_layout_default_reproduces_today() {
        let l = StatusBarLayout::default();
        // location (left), Score/Moves (right), time (right), filter (right).
        assert_eq!(l.segments.len(), 4);
        assert_eq!(l.segments[0].text, "{location}");
        assert!(matches!(l.segments[0].align, Align::Left));
        assert_eq!(l.segments[1].text, "Score: {score}  Moves: {moves}");
        assert!(matches!(l.segments[1].align, Align::Right));
        assert_eq!(l.segments[2].text, "{time}");
        assert_eq!(l.segments[3].text, " {filter}");
        // All built-in segments carry no per-segment override (render in base style).
        assert!(l.segments.iter().all(|s| s.style == Style::default()));
        // ColorScheme carries the default layout.
        assert_eq!(ColorScheme::terminal_default().statusbar_layout, StatusBarLayout::default());
    }

    #[test]
    fn terminal_default_transcript_category_styles() {
        // SQ-0309: these categories are dead ColorScheme fields now; the values
        // ride the theme (registry defaults per docs/design/2026-07-14-styling-
        // role-redesign.md §2's element table).
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.theme.get("transcript_input").style.fg, Some(Color::Cyan));
        assert_eq!(cs.theme.get("transcript_meta").style.fg, Some(Color::DarkGray));
        assert_eq!(cs.theme.get("transcript_warning").style.fg, Some(Color::Yellow));
        // transcript_location = accent (no bold) — a deliberate SQ-0309 redesign
        // change from the old bold-only-white location header.
        assert_eq!(cs.theme.get("transcript_location").style.fg, Some(Color::Cyan));
        assert!(!cs.theme.get("transcript_location").style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(cs.theme.get("transcript_system").style.fg, Some(Color::DarkGray));
        assert_eq!(cs.theme.get("warning_marker").style.fg, Some(Color::Yellow));
        assert!(cs.transcript_rules.is_empty());
    }

    #[test]
    fn resolve_story_style_precedence_and_patch() {
        use ratatui::style::{Color, Modifier};
        let mut cs = ColorScheme::terminal_default(); // transcript fg = White
        // A user rule that only sets bold (no fg) → patch keeps base fg.
        cs.transcript_rules.push(CompiledRule {
            pattern: "^>".into(),
            regex: regex::Regex::new("^>").unwrap(),
            style: Style::new().add_modifier(Modifier::BOLD),
        });

        let base = cs.transcript;

        // 1. User rule wins, patch semantics: bold added, base White fg kept.
        let s = cs.resolve_story_style(base, "> go north", Some("West of House"), false);
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert_eq!(s.fg, Some(Color::White));

        // 2. Built-in location: line equals room name → transcript_location's
        // accent colour patched over the base (SQ-0309: accent, not bold).
        let loc = cs.resolve_story_style(base, "West of House", Some("West of House"), false);
        assert_eq!(loc.fg, Some(Color::Cyan));
        assert!(!loc.add_modifier.contains(Modifier::BOLD));

        // 2b. Boundary guard: "Hall" line vs room "Hallway" must NOT match location.
        let no_loc = cs.resolve_story_style(base, "Hall", Some("Hallway"), false);
        assert_eq!(no_loc.fg, Some(Color::White));
        assert_eq!(no_loc, cs.transcript); // falls through to base

        // 3. Built-in system: bracketed line → transcript_system (DarkGray).
        let sys =
            cs.resolve_story_style(base, "[Your score just went up by ten points.]", None, false);
        assert_eq!(sys.fg, Some(Color::DarkGray));

        // 4. No match → base transcript.
        assert_eq!(cs.resolve_story_style(base, "plain prose", None, false), cs.transcript);

        // 5. None room name → location never matches.
        assert_eq!(cs.resolve_story_style(base, "West of House", None, false), cs.transcript);
    }

    /// SQ-0822: under ZMSD §8.3's Amiga interpreter the machine owns the screen's
    /// one pair of pens, and the built-in "bracketed line = a message from the
    /// interpreter" guess is withdrawn — the line is the game's prose, and muting
    /// it paints dark grey on the Amiga's dark-grey page. The two rules either
    /// side of it are untouched.
    #[test]
    fn the_machine_ink_withdraws_the_built_in_system_rule_and_nothing_else() {
        use ratatui::style::{Color, Modifier};
        let mut cs = ColorScheme::terminal_default();
        cs.transcript_rules.push(CompiledRule {
            pattern: "^>".into(),
            regex: regex::Regex::new("^>").unwrap(),
            style: Style::new().add_modifier(Modifier::BOLD),
        });
        // The Amiga's own pair, as `v6_machine_page` would have laid it under the
        // base style: white ink on the dark grey of colour 12.
        let base = cs.transcript.fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(66, 66, 66));
        let notice = "[You have earned ten chivalry points.]";

        let sys = cs.resolve_story_style(base, notice, None, true);
        assert_eq!(sys, base, "the notice is prose: the machine's ink, the machine's page");
        assert_ne!(
            sys.fg,
            cs.theme.get("transcript_system").style.fg,
            "…and never the muted colour that made it unreadable",
        );
        // Off the Amiga the rule is exactly as it was.
        assert_eq!(
            cs.resolve_story_style(base, notice, None, false).fg,
            cs.theme.get("transcript_system").style.fg,
        );
        // A user rule still wins on either machine, and the location aid still fires.
        assert!(cs
            .resolve_story_style(base, "> go north", None, true)
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(
            cs.resolve_story_style(base, "West of House", Some("West of House"), true).fg,
            cs.theme.get("transcript_location").style.fg,
        );
    }

    #[test]
    fn compiled_rule_eq_ignores_regex_object() {
        let a = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
        let b = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
        assert_eq!(a, b);
    }

    #[test]
    fn parse_color_value_accepts_named_colors() {
        let scheme = GhosttyScheme::default(); // or a minimal scheme
        assert_eq!(parse_color_value("red", &scheme), Some(Color::Red));
        assert_eq!(parse_color_value("bright-blue", &scheme), Some(Color::LightBlue));
        assert_eq!(parse_color_value("white", &scheme), Some(Color::White));
    }

    #[test]
    fn parse_color_value_disambiguates_hex_from_256_index() {
        // SQ-0642: 6-char all-hex-digit strings are hex (the documented hashless
        // form); only a 1-3 digit decimal ≤255 is a 256-index. "000080" used to
        // hit the u8 parse first and misparse as Indexed(80).
        let gs = GhosttyScheme::default();
        assert_eq!(parse_color_value("000080", &gs), Some(Color::Rgb(0, 0, 0x80)));
        assert_eq!(parse_color_value("000000", &gs), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_color_value("80", &gs), Some(Color::Indexed(80)));
        assert_eq!(parse_color_value("255", &gs), Some(Color::Indexed(255)));
        assert_eq!(parse_color_value("ff0000", &gs), Some(Color::Rgb(0xff, 0, 0)));
        // Out-of-range decimals and non-colours stay unparsed.
        assert_eq!(parse_color_value("999", &gs), None);
        assert_eq!(parse_color_value("1234", &gs), None);
    }

    #[test]
    fn from_ghostty_keeps_the_default_border_structure() {
        // SQ-0641: choosing a colour scheme must not change border STRUCTURE.
        // from_ghostty used to seed map/story borders as None (vs terminal
        // default Single), so any scheme dropped the pane borders and re-enabled
        // the in-content map layer strip alongside the header tabs.
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let cs = ColorScheme::from_ghostty(&sample_scheme(), &BTreeMap::new());
        let def = ColorScheme::terminal_default();
        assert_eq!(cs.map_border_style, def.map_border_style);
        assert_eq!(cs.story_border_style, def.story_border_style);
        assert_eq!(cs.map_border_style, BorderStyle::Single);
        assert_eq!(cs.map_border_sides, PaneSides::all(BorderStyle::Single));
        assert_eq!(cs.story_border_sides, PaneSides::all(BorderStyle::Single));
    }

    #[test]
    fn parse_color_value_maps_default_and_reset_to_reset() {
        let gs = GhosttyScheme::default();
        assert_eq!(parse_color_value("default", &gs), Some(Color::Reset));
        assert_eq!(parse_color_value("reset", &gs), Some(Color::Reset));
    }

    // ── GhosttyScheme::parse ──────────────────────────────────────────────────

    const SAMPLE_THEME: &str = r#"
palette = 0=#1d1f21
palette = 1=#cc6666
palette = 6=#70c0ba
palette = 8=#373b41
palette = 15=#ffffff
background = 1d1f21
foreground = c5c8c6
cursor-color = c5c8c6
selection-background = 373b41
selection-foreground = c5c8c6
unknown-key = ignored
"#;

    #[test]
    fn parse_palette_entry() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.palette[1], Color::Rgb(0xcc, 0x66, 0x66));
        assert_eq!(gs.palette[6], Color::Rgb(0x70, 0xc0, 0xba));
        assert_eq!(gs.palette[15], Color::Rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn parse_background_foreground() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.background, Color::Rgb(0x1d, 0x1f, 0x21));
        assert_eq!(gs.foreground, Color::Rgb(0xc5, 0xc8, 0xc6));
    }

    #[test]
    fn parse_optional_fields() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.cursor, Some(Color::Rgb(0xc5, 0xc8, 0xc6)));
        assert_eq!(gs.selection_bg, Some(Color::Rgb(0x37, 0x3b, 0x41)));
        assert_eq!(gs.selection_fg, Some(Color::Rgb(0xc5, 0xc8, 0xc6)));
    }

    #[test]
    fn parse_missing_optional_fields_are_none() {
        let text = "background = 000000\nforeground = ffffff\n";
        let gs = GhosttyScheme::parse(text).unwrap();
        assert!(gs.cursor.is_none());
        assert!(gs.selection_bg.is_none());
        assert!(gs.selection_fg.is_none());
    }

    #[test]
    fn parse_malformed_lines_are_skipped() {
        // The theme has a malformed palette line and an invalid hex; both should be ignored.
        let text = "background = 000000\nforeground = ffffff\npalette = notanumber=#ff0000\npalette = 3=zzzzzz\n";
        let gs = GhosttyScheme::parse(text).unwrap();
        // Malformed entries leave the slot at Reset.
        assert_eq!(gs.palette[3], Color::Reset);
    }

    #[test]
    fn parse_missing_background_is_error() {
        let text = "foreground = ffffff\n";
        assert!(GhosttyScheme::parse(text).is_err());
    }

    #[test]
    fn parse_missing_foreground_is_error() {
        let text = "background = 000000\n";
        assert!(GhosttyScheme::parse(text).is_err());
    }

    #[test]
    fn parse_hex_with_and_without_hash() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Color::Rgb(0xff, 0, 0)));
        assert_eq!(parse_hex_color("ff0000"), Some(Color::Rgb(0xff, 0, 0)));
        assert_eq!(parse_hex_color("zzzzzz"), None);
    }

    // ── ColorScheme::terminal_default ────────────────────────────────────────

    #[test]
    fn terminal_default_carries_a_matching_theme() {
        let cs = ColorScheme::terminal_default();
        // The attached registry theme reproduces the legacy fields it replaced.
        assert_eq!(cs.theme.get("transcript").style.fg, cs.transcript.fg);
        assert_eq!(cs.theme.get("panel.border:active").style.fg, Some(Color::Cyan));
        assert!(cs.theme.get("panel.border:active").style.add_modifier
            .contains(ratatui::style::Modifier::BOLD));
    }

    // ── ColorScheme::from_ghostty ─────────────────────────────────────────────

    fn sample_scheme() -> GhosttyScheme {
        GhosttyScheme::parse(SAMPLE_THEME).unwrap()
    }

    // SQ-0309: `room_selected`/`connector`/`connector_distorted`/`suggestion`/
    // `focused_border`/`status_bar`/`help_bar` are dead ColorScheme fields now
    // (render reads `theme.get("map.*")`/`theme.get("panel.border[:active]")`/
    // etc. instead — see `render/map.rs`, `main.rs`'s `panel_border`). Their
    // resolved-colour coverage lives in `theme::resolve`'s own tests
    // (`resolve_theme_reproduces_terminal_defaults`); only the still-live
    // `transcript` override mechanism is exercised here.

    #[test]
    fn element_override_hex_beats_mapping() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        overrides.insert("transcript".to_string(), "#ff0000".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.transcript.fg, Some(Color::Rgb(0xff, 0, 0)));
    }

    #[test]
    fn element_override_palette_ref_beats_mapping() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        // Override transcript to use palette[1] instead of the scheme foreground.
        overrides.insert("transcript".to_string(), "palette:1".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.transcript, Style::new().fg(gs.palette[1]));
    }

    #[test]
    fn element_override_named_color_works() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        overrides.insert("transcript".to_string(), "cyan".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.transcript, Style::new().fg(Color::Cyan));
    }

    #[test]
    fn sound_beep_defaults_are_amber_and_cyan_blue() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.theme.get("sound_beep_high").style.fg, Some(Color::Rgb(255, 180, 40)));
        assert_eq!(cs.theme.get("sound_beep_low").style.fg, Some(Color::Rgb(60, 140, 220)));
    }

    #[test]
    fn loc_indicator_default_is_dim() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.theme.get("map.loc_indicator").style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn terminal_default_palette_maps_standard_colours_concretely() {
        // Retargeted for SQ-0532/A-F5. ZMSD §8.3.1.1: "The equivalences between
        // the colour numbers and true colours are recommended. The interpreter may
        // allow the user to change the mapping, but the given values should be the
        // default." The built-in default therefore resolves Standard colours to
        // the §8.3.1 RGBs, not to named ANSI colours — which mattered most for
        // white(9), previously `Color::Gray` (the dim VGA base white) in the cell
        // path while the raster path already drew the spec's (255,255,255).
        use zvm::screen::ZColour;
        use ratatui::style::Color;
        let s = ColorScheme::terminal_default();
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(2), &s), Color::Rgb(0, 0, 0), "black (true $0000)");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(3), &s), Color::Rgb(239, 0, 0), "red (true $001D)");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(9), &s), Color::Rgb(255, 255, 255), "white (true $7FFF)");
    }

    #[test]
    fn default_palette_equals_the_zmsd_standard_rgbs() {
        // A-F5: every Standard colour 2..=9 in the DEFAULT scheme is exactly its
        // ZMSD §8.3.1 true-colour equivalent, so a game colour cannot look one way
        // in a terminal cell and another in the v6 pixel canvas.
        let s = ColorScheme::terminal_default();
        for (n, v15) in STANDARD_COLOUR_RGB15 {
            let (r, g, b) = zvm::screen::rgb15_to_888(v15);
            assert_eq!(
                s.palette[(n - 2) as usize],
                Color::Rgb(r, g, b),
                "Standard({n}) must be the §8.3.1 true colour"
            );
        }
    }

    #[test]
    fn cell_and_raster_paths_agree_on_standard_white() {
        // A-F5: the same game colour through the terminal cell path
        // (`resolve_zcolour` → palette) and the pixel path
        // (`v6_layout::standard_pixel_rgb`, exercised here via its shared table).
        let s = ColorScheme::terminal_default();
        let cell = crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(9), &s);
        let raster = standard_colour_rgb(9).expect("white is in the table");
        assert_eq!(cell, Color::Rgb(raster.0, raster.1, raster.2));
        assert_eq!(raster, (255, 255, 255), "§8.3.1: 9 = white (true $7FFF)");
    }

    #[test]
    fn nearest_standard_colour_is_identity_on_the_table() {
        // A-F2: each §8.3.1 true colour maps back to its own colour number.
        for (n, v15) in STANDARD_COLOUR_RGB15 {
            assert_eq!(nearest_standard_colour(zvm::screen::rgb15_to_888(v15)), n);
        }
    }

    #[test]
    fn nearest_standard_colour_snaps_off_table_colours_sensibly() {
        // Near-misses land on the obvious neighbour...
        assert_eq!(nearest_standard_colour((10, 10, 12)), 2, "near-black → black");
        assert_eq!(nearest_standard_colour((250, 250, 240)), 9, "off-white → white");
        assert_eq!(nearest_standard_colour((200, 20, 20)), 3, "dark red → red");
        assert_eq!(nearest_standard_colour((30, 90, 160)), 6, "muted blue → blue");
        // Real terminal themes land where you would expect, both ends.
        assert_eq!(nearest_standard_colour((0x28, 0x2c, 0x34)), 2, "One Dark page → black");
        assert_eq!(nearest_standard_colour((0x00, 0x2b, 0x36)), 2, "Solarized dark page → black");
        assert_eq!(nearest_standard_colour((0xfd, 0xf6, 0xe3)), 9, "Solarized light page → white");
        // A mid-grey is the honest hard case: colours 2..=9 contain NO grey (the
        // greys are 10..=12, Version 6 only, and `set_default_colours` clamps to
        // 2..=9 anyway), so the nearest entry is blue — the only mid-luminance
        // colour in the set — not a coin flip between black and white.
        assert_eq!(nearest_standard_colour((128, 128, 128)), 6, "mid grey → blue (no grey in 2..=9)");
    }

    #[test]
    fn host_default_pair_prefers_a_fully_concrete_theme() {
        // A-F2 / ZMSD §8.3.3: (background, foreground) in $2C/$2D order.
        let dark = Style::new().fg(Color::Rgb(240, 240, 240)).bg(Color::Rgb(8, 8, 12));
        assert_eq!(host_default_colour_pair(dark, None, None), Some((2, 9)),
            "dark page + white ink → black background, white foreground");
        let light = Style::new().fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255));
        assert_eq!(host_default_colour_pair(light, None, None), Some((9, 2)),
            "white page + black ink → white background, black foreground");
    }

    #[test]
    fn host_default_pair_falls_back_to_osc_then_gives_up() {
        // A half-resolved theme is skipped WHOLE (never mixed) — the OSC pair is
        // used instead, mirroring the v6 raster's `v6_default_pair` layering.
        let half = Style::new().fg(Color::Rgb(240, 240, 240));
        assert_eq!(
            host_default_colour_pair(half, Some((0, 0, 0)), Some((255, 255, 255))),
            Some((9, 2)),
            "OSC white page + black ink wins over a theme that set only fg"
        );
        // Neither layer complete → None, so the caller leaves the VM's §8.3.2
        // black-on-white seed alone.
        assert_eq!(host_default_colour_pair(half, None, Some((255, 255, 255))), None);
        assert_eq!(host_default_colour_pair(Style::new(), None, None), None);
        // A non-RGB (named/indexed) theme colour is not a known RGB either.
        let named = Style::new().fg(Color::White).bg(Color::Black);
        assert_eq!(host_default_colour_pair(named, None, None), None);
    }

    // ── SQ-0510: seeding an unconfigured scheme from the OSC 10/11 probe ──────

    fn probe(fg: Option<(u8, u8, u8)>, bg: Option<(u8, u8, u8)>) -> crate::term_colors::TermDefaultColors {
        crate::term_colors::TermDefaultColors {
            fg: fg.map(|(r, g, b)| image::Rgba([r, g, b, 255])),
            bg: bg.map(|(r, g, b)| image::Rgba([r, g, b, 255])),
        }
    }

    #[test]
    fn seed_fills_an_unconfigured_scheme_from_a_complete_probe() {
        // The reported case: no scheme configured, a terminal that answers both
        // queries. fg/bg take the probed RGB so `Roles::from_scheme` stops
        // early-returning to its white-on-BLACK fallback.
        let seeded = seed_scheme_from_terminal(
            GhosttyScheme::default(),
            &probe(Some((0xc5, 0xc8, 0xc6)), Some((0xfd, 0xf6, 0xe3))),
        );
        assert_eq!(seeded.foreground, Color::Rgb(0xc5, 0xc8, 0xc6));
        assert_eq!(seeded.background, Color::Rgb(0xfd, 0xf6, 0xe3));
        // The palette is deliberately untouched: `from_scheme`'s per-slot
        // fallback still supplies the cyan/yellow/grey accents (SQ-0642).
        assert_eq!(seeded.palette, [Color::Reset; 16], "accents keep their per-slot fallback");
    }

    #[test]
    fn seed_is_inert_without_a_complete_probe() {
        // A terminal that answers nothing, or only one channel, must leave the
        // scheme exactly as `resolve_base` handed it over — a half-resolved layer
        // is skipped whole rather than mixed (the `host_default_colour_pair` rule),
        // so nobody ever gets a probed ink on a guessed page.
        let base = GhosttyScheme::default();
        for p in [
            probe(None, None),
            probe(Some((0xc5, 0xc8, 0xc6)), None),
            probe(None, Some((0xfd, 0xf6, 0xe3))),
        ] {
            let seeded = seed_scheme_from_terminal(base.clone(), &p);
            assert_eq!(seeded, base, "an incomplete probe answer changes nothing: {p:?}");
        }
    }

    #[test]
    fn seed_never_second_guesses_a_configured_scheme() {
        // A real scheme (built-in name or file) is the user's explicit choice; the
        // probe must not overwrite either channel of it.
        let gs = GhosttyScheme::parse("background = 1d1f21\nforeground = c5c8c6\n").unwrap();
        let seeded =
            seed_scheme_from_terminal(gs.clone(), &probe(Some((255, 255, 255)), Some((255, 255, 255))));
        assert_eq!(seeded, gs, "a configured scheme speaks for itself");
    }
}
