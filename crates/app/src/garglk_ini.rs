//! garglk.ini auto-importer (SQ-0319): discover a per-game Gargoyle config
//! beside the story, resolve the section matching this story, and overlay its
//! colours onto our [`ColorScheme`] (the SQ-0331 per-Glk-style slots + the
//! element bases). Gargoyle's `garglk.ini` is a tiny whitespace/section format,
//! so we hand-parse it here rather than pull in an INI dependency.
//!
//! Format (verified against Gargoyle):
//! - `key value...` lines; `#` and `;` start comments; blank lines ignored.
//! - `[ selector ... ]` opens a section scoping the following keys. Keys before
//!   any section are GLOBAL. A selector is a game filename (`trinity.z4`), an
//!   extension glob (`*.z5`), or an interpreter name (e.g. `bocfel` — we ignore
//!   those). Multiple space-separated selectors per header; the section applies
//!   if ANY matches. Later matching sections override earlier ones.
//! - Colour values are 6 hex digits (`ffffff`), optional leading `#`.
//!
//! We map: `tcolor N` → buffer (text-buffer) Glk style slot N; `gcolor N` →
//! grid (text-grid) slot N; `tcolor 0`/`gcolor 0` also fold into the element
//! base (`transcript`/`upper_window`); `linkcolor`/`bordercolor`/`windowcolor`
//! → element fields; `stylehint 0|1` → `honor_game_colours`. Fonts/sizes/margins
//! are ignored (no meaning in a terminal) but counted for the startup summary.
//!
//! TODO(SQ-0319 follow-up): user-level / global `garglk.ini` (config dir) is out
//! of scope; a `/import-garglk <path>` command could cover explicit imports.

use std::path::{Path, PathBuf};

use ratatui::style::Color;

use crate::colors::ColorScheme;

const NUM_STYLES: usize = 11;

/// A garglk config resolved for one story: the effective keys (global keys plus
/// every matching section, merged in file order) ready to overlay onto a
/// [`ColorScheme`]. Kept in `AppState` so it can be re-applied after any
/// `reload_style` (which rebuilds `state.colors` from scratch).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GarglkOverlay {
    /// The sidecar file the overlay was read from (for the startup summary).
    pub path: PathBuf,
    /// Section selectors that matched this story (empty = only global keys).
    pub matched: Vec<String>,
    /// `tcolor N` → buffer Glk style slot N: `Some((fg, bg))`.
    pub buffer: [Option<(Color, Color)>; NUM_STYLES],
    /// `gcolor N` → grid Glk style slot N.
    pub grid: [Option<(Color, Color)>; NUM_STYLES],
    /// `linkcolor` → hyperlink foreground.
    pub linkcolor: Option<Color>,
    /// `bordercolor` → upper-window border (and window separator) foreground.
    pub bordercolor: Option<Color>,
    /// `windowcolor` → pane background.
    pub windowcolor: Option<Color>,
    /// `stylehint 0|1` → `honor_game_colours` (false/true).
    pub honor_game_colours: Option<bool>,
    /// `tmarginx` (px) → text-buffer inner margin in columns (SQ-0344), converted
    /// via [`px_to_cells`]. `None` = no `tmarginx`, so the config default stands.
    pub margin_x: Option<u16>,
    /// `tmarginy` (px) → text-buffer inner margin in rows (SQ-0344).
    pub margin_y: Option<u16>,
    /// `wborderx`/`wbordery` (px inter-window border width) → borderless toggle
    /// (SQ-0344): width `0` → `Some(true)` (no separator), `>0` → `Some(false)`.
    /// `None` = the ini set no border width, so the per-game/default choice stands.
    pub borderless: Option<bool>,
    /// Count of ignored font/size keys (for the summary line only).
    pub ignored_font_keys: usize,
}

/// Convert a garglk pixel margin to terminal cells (SQ-0344) using a nominal cell
/// size — 8px wide, 16px tall — rounded to the nearest cell and capped at 8 so a
/// large pixel margin can't swallow the pane. garglk's defaults (`tmargin` 7px)
/// land at ≈1 column / 0 rows.
const CELL_W_PX: u16 = 8;
const CELL_H_PX: u16 = 16;
fn px_to_cells(px: u16, cell_px: u16) -> u16 {
    ((px + cell_px / 2) / cell_px).min(8)
}

/// What [`GarglkOverlay::apply`] changed, for the one-line startup summary.
pub struct GarglkSummary {
    pub path: PathBuf,
    pub matched: Vec<String>,
    pub buffer_count: usize,
    pub grid_count: usize,
    pub link_set: bool,
    pub border_set: bool,
    pub window_set: bool,
    pub honor: Option<bool>,
    pub margin: Option<(u16, u16)>,
    pub borderless: Option<bool>,
    pub ignored_font_keys: usize,
}

impl GarglkSummary {
    /// Render the `lanthorn: imported garglk config …` startup line.
    pub fn console_line(&self) -> String {
        let section = if self.matched.is_empty() {
            "global keys".to_string()
        } else {
            format!("section '{}'", self.matched.join(" "))
        };
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "{} buffer + {} grid style colours",
            self.buffer_count, self.grid_count
        ));
        let mut els: Vec<&str> = Vec::new();
        if self.link_set {
            els.push("link");
        }
        if self.border_set {
            els.push("border");
        }
        if self.window_set {
            els.push("window");
        }
        if !els.is_empty() {
            parts.push(format!("{} set", els.join("/")));
        }
        if let Some(h) = self.honor {
            parts.push(format!("honor_game_colours={h}"));
        }
        if let Some((mx, my)) = self.margin {
            parts.push(format!("text margin {mx}×{my} cells"));
        }
        if let Some(b) = self.borderless {
            parts.push(format!("borders {}", if b { "off" } else { "on" }));
        }
        let tail = if self.ignored_font_keys > 0 {
            format!(" ({} font/size keys ignored)", self.ignored_font_keys)
        } else {
            String::new()
        };
        format!(
            "imported garglk config {} ({}): {}{}",
            self.path.display(),
            section,
            parts.join(", "),
            tail
        )
    }
}

impl GarglkOverlay {
    /// Build the theme's garglk [`Decls`](crate::theme::resolve::Decls) layer from
    /// this overlay's non-glk colours (SQ-0309): `tcolor 0`→`transcript`,
    /// `gcolor 0`→`upper_window`, `linkcolor`→`hyperlink`, `bordercolor`→
    /// `upper_window_border`, and `windowcolor`→the page background on
    /// `transcript`/`upper_window`. The per-style glk slots (`buffer[1..]`/
    /// `grid[1..]`) still ride the `glk_styles` override array via [`Self::apply`]
    /// (the render glk path reads that array for colours), so they are NOT
    /// duplicated here. Fed to `resolve_theme_layered` as the garglk layer so these
    /// colours survive the render migration off the legacy `ColorScheme` fields.
    pub fn color_decls(&self) -> crate::theme::resolve::Decls {
        use crate::theme::registry::Delta;
        let mut d = crate::theme::resolve::Decls::new();
        let mut set = |name: &str, fg: Option<Color>, bg: Option<Color>| {
            let e = d.entry(name.to_string()).or_insert(Delta::EMPTY);
            if fg.is_some() {
                e.fg = fg;
            }
            if bg.is_some() {
                e.bg = bg;
            }
        };
        if let Some((fg, bg)) = self.buffer[0] {
            set("transcript", Some(fg), Some(bg));
        }
        if let Some((fg, bg)) = self.grid[0] {
            set("upper_window", Some(fg), Some(bg));
        }
        if let Some(c) = self.linkcolor {
            set("hyperlink", Some(c), None);
        }
        if let Some(c) = self.bordercolor {
            set("upper_window_border", Some(c), None);
        }
        if let Some(c) = self.windowcolor {
            // `windowcolor` is the generic page background; an explicit `tcolor 0`
            // / `gcolor 0` background (Normal style) is more specific and wins, so
            // only fall back to `windowcolor` where that slot didn't set a bg.
            if self.buffer[0].is_none() {
                set("transcript", None, Some(c));
            }
            if self.grid[0].is_none() {
                set("upper_window", None, Some(c));
            }
        }
        d
    }

    /// Overlay the resolved keys onto `cs`, returning a summary of what changed.
    /// Starts from the already-resolved theme and only touches what the ini sets.
    /// `honor_game_colours` is NOT applied here (it lives on `config`, set by the
    /// caller from [`Self::honor_game_colours`]).
    pub fn apply(&self, cs: &mut ColorScheme) -> GarglkSummary {
        let mut buffer_count = 0;
        let mut grid_count = 0;

        for (n, slot) in self.buffer.iter().enumerate() {
            if let Some((fg, bg)) = *slot {
                buffer_count += 1;
                if n == 0 {
                    // Slot 0 (Normal) stays None per SQ-0331 — Normal IS the
                    // element, so fold it into the buffer element base instead.
                    cs.transcript = cs.transcript.fg(fg).bg(bg);
                } else {
                    cs.glk_styles[0][n] = ratatui::style::Style {
                        fg: Some(fg),
                        bg: Some(bg),
                        ..ratatui::style::Style::default()
                    };
                }
            }
        }
        for (n, slot) in self.grid.iter().enumerate() {
            if let Some((fg, bg)) = *slot {
                grid_count += 1;
                // Slot 0 (Normal) folds into the grid element base
                // (`upper_window`) — SQ-0309: that base is now theme-only
                // (`color_decls` feeds it into the theme's garglk layer), so
                // there is no legacy field left to write here.
                if n != 0 {
                    cs.glk_styles[1][n] = ratatui::style::Style {
                        fg: Some(fg),
                        bg: Some(bg),
                        ..ratatui::style::Style::default()
                    };
                }
            }
        }

        // `linkcolor`/`bordercolor`/the grid half of `windowcolor` are SQ-0309
        // dead legacy-field writes: `hyperlink`/`upper_window_border`/
        // `upper_window` reach the render path only through the theme now
        // (`color_decls`, above), which already covers all three.
        if let Some(c) = self.windowcolor {
            // Paint the buffer page background (the grid half rides the theme).
            cs.transcript = cs.transcript.bg(c);
        }

        GarglkSummary {
            path: self.path.clone(),
            matched: self.matched.clone(),
            buffer_count,
            grid_count,
            link_set: self.linkcolor.is_some(),
            border_set: self.bordercolor.is_some(),
            window_set: self.windowcolor.is_some(),
            honor: self.honor_game_colours,
            margin: match (self.margin_x, self.margin_y) {
                (None, None) => None,
                (mx, my) => Some((mx.unwrap_or(0), my.unwrap_or(0))),
            },
            borderless: self.borderless,
            ignored_font_keys: self.ignored_font_keys,
        }
    }
}

/// Discover the per-game garglk sidecar beside `story_path`, highest priority
/// first: `<storystem>.ini`, then `garglk.ini`. Parse the first that exists and
/// resolve the section set matching this story. Returns `None` if neither file
/// exists (or the match yields nothing to apply).
pub fn discover(story_path: &Path) -> Option<GarglkOverlay> {
    let dir = story_path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let stem = story_path.file_stem()?.to_string_lossy().to_string();

    let candidates = [dir.join(format!("{stem}.ini")), dir.join("garglk.ini")];
    for cand in candidates {
        if cand.is_file() {
            let text = std::fs::read_to_string(&cand).ok()?;
            return Some(parse_for_story(&text, story_path, cand));
        }
    }
    None
}

/// A parsed `[selector ...]` block (or the leading global block with empty
/// `selectors`) and its ordered key/value lines.
struct Block {
    selectors: Vec<String>,
    keys: Vec<(String, Vec<String>)>,
}

/// Parse the ini text into ordered blocks. The leading block (before any `[…]`)
/// has empty `selectors` and holds the global keys.
fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = vec![Block { selectors: Vec::new(), keys: Vec::new() }];
    for raw in text.lines() {
        // Strip inline comments (`#`/`;`) and trim.
        let line = raw
            .split(['#', ';'])
            .next()
            .unwrap_or("")
            .trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let selectors: Vec<String> = inner.split_whitespace().map(|s| s.to_string()).collect();
            blocks.push(Block { selectors, keys: Vec::new() });
        } else {
            let mut it = line.split_whitespace();
            if let Some(key) = it.next() {
                let args: Vec<String> = it.map(|s| s.to_string()).collect();
                blocks
                    .last_mut()
                    .expect("at least the global block exists")
                    .keys
                    .push((key.to_ascii_lowercase(), args));
            }
        }
    }
    blocks
}

/// Does `selector` match this story? A `*.ext` glob matches the story's
/// extension; a name containing `.` matches the exact filename; anything else is
/// treated as an interpreter name and ignored (never matches).
fn selector_matches(selector: &str, filename: &str, extension: &str) -> bool {
    if let Some(ext) = selector.strip_prefix("*.") {
        !ext.is_empty() && ext.eq_ignore_ascii_case(extension)
    } else if selector.contains('.') {
        selector.eq_ignore_ascii_case(filename)
    } else {
        // Interpreter selector (e.g. `bocfel`, `git`) — we don't expose those.
        false
    }
}

/// Parse `text` and resolve the effective overlay for `story_path`: global keys
/// plus every section whose selector matches, applied in file order.
fn parse_for_story(text: &str, story_path: &Path, path: PathBuf) -> GarglkOverlay {
    let filename = story_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let extension = story_path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut ov = GarglkOverlay { path, ..GarglkOverlay::default() };

    for block in parse_blocks(text) {
        // The global block (empty selectors) always applies; a section applies
        // if ANY of its selectors matches this story.
        let matched_sel: Option<&String> = if block.selectors.is_empty() {
            None
        } else {
            match block
                .selectors
                .iter()
                .find(|s| selector_matches(s, &filename, &extension))
            {
                Some(s) => Some(s),
                None => continue, // no selector matched — skip this section
            }
        };
        if let Some(sel) = matched_sel {
            if !ov.matched.contains(sel) {
                ov.matched.push(sel.clone());
            }
        }
        for (key, args) in &block.keys {
            apply_key(&mut ov, key, args);
        }
    }

    ov
}

/// Apply one resolved `key args…` line onto the overlay (later lines override).
fn apply_key(ov: &mut GarglkOverlay, key: &str, args: &[String]) {
    match key {
        "tcolor" | "gcolor" => {
            // `tcolor N FG BG`
            if args.len() >= 3 {
                if let (Ok(n), Some(fg), Some(bg)) =
                    (args[0].parse::<usize>(), parse_hex(&args[1]), parse_hex(&args[2]))
                {
                    if n < NUM_STYLES {
                        if key == "tcolor" {
                            ov.buffer[n] = Some((fg, bg));
                        } else {
                            ov.grid[n] = Some((fg, bg));
                        }
                    }
                }
            }
        }
        "linkcolor" => {
            if let Some(c) = args.first().and_then(|a| parse_hex(a)) {
                ov.linkcolor = Some(c);
            }
        }
        "bordercolor" => {
            if let Some(c) = args.first().and_then(|a| parse_hex(a)) {
                ov.bordercolor = Some(c);
            }
        }
        "windowcolor" => {
            if let Some(c) = args.first().and_then(|a| parse_hex(a)) {
                ov.windowcolor = Some(c);
            }
        }
        "stylehint" => {
            match args.first().map(|s| s.as_str()) {
                Some("0") => ov.honor_game_colours = Some(false),
                Some("1") => ov.honor_game_colours = Some(true),
                _ => {}
            }
        }
        // Text-window inner margin (SQ-0344): garglk pixels → terminal cells.
        "tmarginx" => {
            if let Some(px) = args.first().and_then(|a| a.parse::<u16>().ok()) {
                ov.margin_x = Some(px_to_cells(px, CELL_W_PX));
            }
        }
        "tmarginy" => {
            if let Some(px) = args.first().and_then(|a| a.parse::<u16>().ok()) {
                ov.margin_y = Some(px_to_cells(px, CELL_H_PX));
            }
        }
        // Inter-window border width (SQ-0344): 0 → borderless, >0 → bordered.
        "wborderx" | "wbordery" => {
            if let Some(px) = args.first().and_then(|a| a.parse::<u16>().ok()) {
                ov.borderless = Some(px == 0);
            }
        }
        // Remaining font / size / outer-frame keys have no terminal meaning —
        // ignore but count. (`wmarginx/y` is the outer-frame margin, a different
        // concept from the text-window `tmargin` handled above; out of scope.)
        "tfont" | "gfont" | "monofont" | "propfont" | "monor" | "monob" | "monoi" | "monoz"
        | "propr" | "propb" | "propi" | "propz" | "monosize" | "propsize" | "monoaspect"
        | "propaspect" | "leading" | "baseline" | "lcd" | "lcdfilter" | "lcdweights"
        | "wmarginx" | "wmarginy" | "gutterx" | "guttery"
        | "cols" | "rows" | "cellw" | "cellh" => {
            ov.ignored_font_keys += 1;
        }
        // Other keys (morecolor/caretcolor, interpreter opts, unknowns): ignore,
        // uncounted — the summary only advertises font/size keys.
        _ => {}
    }
}

/// Parse a 6-hex-digit RGB value (optional leading `#`) to a concrete
/// [`Color::Rgb`]. Returns `None` on any malformed input.
fn parse_hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(all(test, feature = "t-persist"))]
mod tests {
    use super::*;

    fn story(name: &str) -> PathBuf {
        PathBuf::from(format!("/games/{name}"))
    }

    fn resolve(text: &str, story_name: &str) -> GarglkOverlay {
        parse_for_story(text, &story(story_name), PathBuf::from("garglk.ini"))
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(parse_hex("00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_hex("#0011ff"), Some(Color::Rgb(0, 17, 255)));
        assert_eq!(parse_hex("fff"), None);
        assert_eq!(parse_hex("gggggg"), None);
    }

    #[test]
    fn global_keys_apply_to_every_story() {
        let ov = resolve("tcolor 0 ffffff 000000\nlinkcolor 00ffff\n", "zork1.z3");
        assert_eq!(ov.buffer[0], Some((Color::Rgb(255, 255, 255), Color::Rgb(0, 0, 0))));
        assert_eq!(ov.linkcolor, Some(Color::Rgb(0, 255, 255)));
        assert!(ov.matched.is_empty(), "global-only match has no section selector");
    }

    #[test]
    fn extension_glob_matches_by_extension() {
        let ini = "[*.z5]\ntcolor 3 00ff00 000000\n";
        let hit = resolve(ini, "trinity.z5");
        assert_eq!(hit.buffer[3], Some((Color::Rgb(0, 255, 0), Color::Rgb(0, 0, 0))));
        assert_eq!(hit.matched, vec!["*.z5".to_string()]);

        let miss = resolve(ini, "trinity.z3");
        assert_eq!(miss.buffer[3], None);
        assert!(miss.matched.is_empty());
    }

    #[test]
    fn filename_selector_matches_exact_name() {
        let ini = "[trinity.z4]\nbordercolor ff0000\n";
        assert_eq!(resolve(ini, "trinity.z4").bordercolor, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(resolve(ini, "zork1.z4").bordercolor, None);
    }

    #[test]
    fn interpreter_selectors_are_ignored() {
        // `bocfel` has no `.` and no `*.` — treated as an interpreter, never matches.
        let ini = "[bocfel]\ntcolor 1 112233 445566\n";
        assert_eq!(resolve(ini, "zork1.z3").buffer[1], None);
    }

    #[test]
    fn later_sections_override_earlier() {
        let ini = "\
[*.z5]
tcolor 0 111111 222222
[trinity.z5]
tcolor 0 aaaaaa bbbbbb
";
        let ov = resolve(ini, "trinity.z5");
        assert_eq!(ov.buffer[0], Some((Color::Rgb(0xaa, 0xaa, 0xaa), Color::Rgb(0xbb, 0xbb, 0xbb))));
        assert_eq!(ov.matched, vec!["*.z5".to_string(), "trinity.z5".to_string()]);
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let ini = "\
# a comment
; another
tcolor 2 010203 040506   # trailing comment

linkcolor 070809 ; trailing
";
        let ov = resolve(ini, "a.z5");
        assert_eq!(ov.buffer[2], Some((Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6))));
        assert_eq!(ov.linkcolor, Some(Color::Rgb(7, 8, 9)));
    }

    #[test]
    fn stylehint_gates_honor_game_colours() {
        assert_eq!(resolve("stylehint 0\n", "a.z5").honor_game_colours, Some(false));
        assert_eq!(resolve("stylehint 1\n", "a.z5").honor_game_colours, Some(true));
        assert_eq!(resolve("tcolor 0 000000 ffffff\n", "a.z5").honor_game_colours, None);
    }

    #[test]
    fn font_and_size_keys_counted_as_ignored() {
        let ini = "tfont Serif\ngfont Mono\nmonosize 12\nlinkcolor 00ff00\n";
        let ov = resolve(ini, "a.z5");
        assert_eq!(ov.ignored_font_keys, 3);
        assert_eq!(ov.linkcolor, Some(Color::Rgb(0, 255, 0)));
    }

    #[test]
    fn apply_maps_slots_and_elements() {
        let ini = "\
tcolor 0 ffffff 000000
tcolor 3 00ff00 000000
gcolor 0 aabbcc 001122
linkcolor 00ffff
bordercolor ff8800
windowcolor 101010
stylehint 0
";
        let ov = resolve(ini, "a.z5");
        let mut cs = ColorScheme::terminal_default();
        let summary = ov.apply(&mut cs);

        // tcolor 3 → buffer Glk slot 3 (both channels concrete).
        assert_eq!(
            cs.glk_styles[0][3],
            ratatui::style::Style {
                fg: Some(Color::Rgb(0, 255, 0)),
                bg: Some(Color::Rgb(0, 0, 0)),
                ..ratatui::style::Style::default()
            }
        );
        // tcolor 0 → element base (slot 0 stays None).
        assert_eq!(cs.glk_styles[0][0], ratatui::style::Style::default());
        assert_eq!(cs.transcript.fg, Some(Color::Rgb(255, 255, 255)));
        // windowcolor overrides the buffer background (applied after tcolor 0 bg).
        assert_eq!(cs.transcript.bg, Some(Color::Rgb(0x10, 0x10, 0x10)));
        // gcolor 0 → grid slot 0 stays None too (its colour now rides the theme
        // only — see `color_decls_maps_non_glk_colours_to_theme_selectors`).
        assert_eq!(cs.glk_styles[1][0], ratatui::style::Style::default());
        // linkcolor / bordercolor are theme-only now (SQ-0309): no legacy
        // `hyperlink`/`upper_window_border` field left to assert on here.

        assert_eq!(summary.buffer_count, 2);
        assert_eq!(summary.grid_count, 1);
        assert!(summary.link_set && summary.border_set && summary.window_set);
        assert_eq!(summary.honor, Some(false));
    }

    #[test]
    fn color_decls_maps_non_glk_colours_to_theme_selectors() {
        // The same overlay's non-glk colours must reach the theme layer (SQ-0309),
        // since the render sites now read those from the theme, not the fields.
        let ov = resolve(
            "tcolor 0 ffffff 000000\ngcolor 0 aabbcc 001122\nlinkcolor 00ffff\nbordercolor ff8800\nwindowcolor 101010\n",
            "a.z5",
        );
        let d = ov.color_decls();
        assert_eq!(d["transcript"].fg, Some(Color::Rgb(0xff, 0xff, 0xff)));
        // tcolor 0's explicit bg wins over the generic windowcolor (precedence).
        assert_eq!(d["transcript"].bg, Some(Color::Rgb(0x00, 0x00, 0x00)));
        assert_eq!(d["upper_window"].fg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
        // gcolor 0's explicit bg likewise wins over windowcolor.
        assert_eq!(d["upper_window"].bg, Some(Color::Rgb(0x00, 0x11, 0x22)));
        assert_eq!(d["hyperlink"].fg, Some(Color::Rgb(0, 0xff, 0xff)));
        assert_eq!(d["upper_window_border"].fg, Some(Color::Rgb(0xff, 0x88, 0)));
    }

    #[test]
    fn windowcolor_paints_buffer_background() {
        // SQ-0309: `apply()` no longer touches the grid background at all (that
        // now rides `color_decls` into the theme exclusively — see
        // `color_decls_maps_non_glk_colours_to_theme_selectors`); it still
        // paints the buffer (`transcript`) background directly.
        let ov = resolve("gcolor 0 ffffff 010203\nwindowcolor 0a0b0c\n", "a.z5");
        let mut cs = ColorScheme::terminal_default();
        ov.apply(&mut cs);
        assert_eq!(cs.transcript.bg, Some(Color::Rgb(0x0a, 0x0b, 0x0c)));
    }

    #[test]
    fn console_line_reads_cleanly() {
        let summary = GarglkSummary {
            path: PathBuf::from("/games/garglk.ini"),
            matched: vec!["*.z5".to_string()],
            buffer_count: 4,
            grid_count: 2,
            link_set: true,
            border_set: true,
            window_set: true,
            honor: Some(false),
            margin: None,
            borderless: None,
            ignored_font_keys: 3,
        };
        assert_eq!(
            summary.console_line(),
            "imported garglk config /games/garglk.ini (section '*.z5'): \
             4 buffer + 2 grid style colours, link/border/window set, \
             honor_game_colours=false (3 font/size keys ignored)"
        );
    }

    #[test]
    fn tmargin_pixels_convert_to_cells_and_report() {
        // garglk defaults (tmargin 7px) → ≈1 column / 0 rows with the nominal cell.
        let ov = resolve("tmarginx 7\ntmarginy 7\n", "a.z5");
        assert_eq!(ov.margin_x, Some(1), "7px / 8px-cell rounds to 1 column");
        assert_eq!(ov.margin_y, Some(0), "7px / 16px-cell rounds to 0 rows");
        assert_eq!(ov.ignored_font_keys, 0, "tmargin keys are handled, not ignored");

        // A large margin scales up and is capped at 8 cells.
        let big = resolve("tmarginx 40\ntmarginy 200\n", "a.z5");
        assert_eq!(big.margin_x, Some(5), "40px / 8 = 5 columns");
        assert_eq!(big.margin_y, Some(8), "200px / 16 = 12 → capped at 8 rows");

        let mut cs = ColorScheme::terminal_default();
        let s = ov.apply(&mut cs);
        assert_eq!(s.margin, Some((1, 0)));
    }

    #[test]
    fn wborder_maps_to_borderless_toggle() {
        // width 0 → borderless; > 0 → bordered. Only explicit keys act.
        assert_eq!(resolve("wborderx 0\n", "a.z5").borderless, Some(true));
        assert_eq!(resolve("wborderx 2\n", "a.z5").borderless, Some(false));
        assert_eq!(resolve("linkcolor 00ff00\n", "a.z5").borderless, None, "no wborder key → no opinion");
    }

    #[test]
    fn discover_prefers_stem_ini_over_garglk_ini() {
        let dir = std::env::temp_dir().join(format!("garglk-disco-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let story = dir.join("zork1.z3");
        std::fs::write(&story, b"dummy").unwrap();
        std::fs::write(dir.join("zork1.ini"), "linkcolor 010101\n").unwrap();
        std::fs::write(dir.join("garglk.ini"), "linkcolor 020202\n").unwrap();

        let ov = discover(&story).expect("stem sidecar found");
        assert_eq!(ov.linkcolor, Some(Color::Rgb(1, 1, 1)), "zork1.ini wins over garglk.ini");
        assert!(ov.path.ends_with("zork1.ini"));

        // Remove the stem sidecar → falls back to garglk.ini.
        std::fs::remove_file(dir.join("zork1.ini")).unwrap();
        let ov2 = discover(&story).expect("garglk.ini fallback");
        assert_eq!(ov2.linkcolor, Some(Color::Rgb(2, 2, 2)));

        // Remove both → nothing to import.
        std::fs::remove_file(dir.join("garglk.ini")).unwrap();
        assert!(discover(&story).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
