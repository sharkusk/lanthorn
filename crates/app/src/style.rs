//! Style model: per-declaration color + modifier parsing.
//!
//! This module owns the partial/raw style representation used by the style-file
//! subsystem. A [`Decl`] is a single CSS-ish declaration block (one selector's
//! worth of properties). [`decl_to_style`] resolves it into a ratatui [`Style`].

use std::collections::BTreeMap;

use ratatui::style::{Modifier, Style};

use crate::colors::{self, ColorScheme};
use crate::render::paneframe;

// ── Decl ──────────────────────────────────────────────────────────────────────

/// A partial style declaration: every field is `Option` so unset fields are
/// distinguished from explicitly set ones.
///
/// The `style` field is only meaningful for border selectors (`map_border`,
/// `story_border`, `status_header`, `input_line`); it is ignored for other selectors.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct Decl {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub dim: Option<bool>,
    pub reversed: Option<bool>,
    /// Optional border-style name (e.g. `"single"`, `"double"`, etc.).
    /// Only interpreted for border selectors; ignored for others.
    #[serde(default)]
    pub style: Option<String>,
    /// Per-side border overrides (border selectors only): each names a line style
    /// (none/single/double/thick). A side falls back to `style` when unset.
    #[serde(default)]
    pub style_top: Option<String>,
    #[serde(default)]
    pub style_bottom: Option<String>,
    #[serde(default)]
    pub style_left: Option<String>,
    #[serde(default)]
    pub style_right: Option<String>,
    /// Whether the pane's header strip is shown (story_border / map_border only).
    #[serde(default)]
    pub header: Option<bool>,
    /// Optional shadow flag. Only interpreted for the `dialog` selector.
    #[serde(default)]
    pub shadow: Option<bool>,
    /// Optional placement token (center/top/bottom/left/right/corners). Only
    /// interpreted for the `dialog` selector.
    #[serde(default)]
    pub placement: Option<String>,
    /// Optional placement margin (cells from the anchored edge). Only interpreted
    /// for the `dialog` selector.
    #[serde(default)]
    pub margin: Option<u16>,
    /// Per-side/corner glyph overrides (border selectors only).
    #[serde(default)]
    pub glyph_top: Option<String>,
    #[serde(default)]
    pub glyph_bottom: Option<String>,
    #[serde(default)]
    pub glyph_left: Option<String>,
    #[serde(default)]
    pub glyph_right: Option<String>,
    #[serde(default)]
    pub glyph_tl: Option<String>,
    #[serde(default)]
    pub glyph_tr: Option<String>,
    #[serde(default)]
    pub glyph_bl: Option<String>,
    #[serde(default)]
    pub glyph_br: Option<String>,
}

// ── decl_to_style ─────────────────────────────────────────────────────────────

/// Convert a [`Decl`] into a ratatui [`Style`].
///
/// - `fg`/`bg` are parsed via [`colors::parse_color_value`].
/// - Each modifier bool adds its modifier when `Some(true)`.
pub fn decl_to_style(d: &Decl, scheme: &colors::GhosttyScheme) -> Style {
    let mut s = Style::new();

    if let Some(ref fg_str) = d.fg {
        if let Some(c) = colors::parse_color_value(fg_str, scheme) {
            s = s.fg(c);
        }
    }

    if let Some(ref bg_str) = d.bg {
        if let Some(c) = colors::parse_color_value(bg_str, scheme) {
            s = s.bg(c);
        }
    }

    if d.bold == Some(true) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if d.italic == Some(true) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if d.underline == Some(true) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    if d.dim == Some(true) {
        s = s.add_modifier(Modifier::DIM);
    }
    if d.reversed == Some(true) {
        s = s.add_modifier(Modifier::REVERSED);
    }

    s
}

// ── StyleSymbols ──────────────────────────────────────────────────────────────

/// Partial symbol configuration from a style file's `[map]` section.
///
/// Every preset field is `Option` so unset fields are distinguished from
/// explicitly set ones. [`finalize_symbols`] fills `None` fields with the
/// existing `config::default_*` values to produce a concrete [`config::SymbolConfig`](crate::config::SymbolConfig).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct StyleSymbols {
    pub box_style: Option<String>,
    pub arrow_set: Option<String>,
    pub portal_icons: Option<String>,
    pub path_style: Option<String>,
    /// Line-art preset for the up/down/in/out portal connectors, styled
    /// separately from the cardinal `path_style`.
    pub portal_path_style: Option<String>,
    /// Glyph preset for the pane-border toggle controls (SQ-1123): "plain" or
    /// "nerdfont".
    pub control_icons: Option<String>,
    /// Glyph preset for the story picker's row badges (SQ-1159): "plain" or
    /// "nerdfont". The `badge_*` keys below layer over whatever it resolves to,
    /// so a player can take the patched set and still spell one badge their own
    /// way.
    pub badge_icons: Option<String>,
    pub badge_save: Option<String>,
    pub badge_hint: Option<String>,
    pub badge_hint_available: Option<String>,
    /// Draw diagonal stubs out of room corners for ne/nw/se/sw exits (SQ-0314).
    /// `None` → the config default (on). Set false for a font without Unicode 13
    /// Legacy Computing coverage.
    pub diagonal_corners: Option<bool>,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

// ── finalize_symbols ──────────────────────────────────────────────────────────

/// Resolve a partial [`StyleSymbols`] into a concrete [`config::SymbolConfig`](crate::config::SymbolConfig).
///
/// Each `None` preset is filled with the existing `config::default_*` value.
/// The `overrides` map is copied as-is.
///
/// The `badge_*` fields are the one two-stage resolution here (SQ-1159):
/// `badge_icons` names a [`crate::symbols::StoryBadges`] preset, and each badge
/// key that IS set overrides that preset's glyph for its own slot. An unknown
/// preset name falls back to the letters, the same way every other category
/// treats one.
pub fn finalize_symbols(s: &StyleSymbols) -> crate::config::SymbolConfig {
    let badge_icons = s.badge_icons.clone().unwrap_or_else(crate::config::default_badge_icons);
    let badges = crate::symbols::StoryBadges::preset(&badge_icons)
        .unwrap_or(crate::symbols::StoryBadges::PLAIN);
    crate::config::SymbolConfig {
        box_style: s.box_style.clone().unwrap_or_else(crate::config::default_box_style),
        arrow_set: s.arrow_set.clone().unwrap_or_else(crate::config::default_arrow_set),
        portal_icons: s.portal_icons.clone().unwrap_or_else(crate::config::default_portal_icons),
        path_style: s.path_style.clone().unwrap_or_else(crate::config::default_path_style),
        portal_path_style: s.portal_path_style.clone().unwrap_or_else(crate::config::default_portal_path_style),
        control_icons: s.control_icons.clone().unwrap_or_else(crate::config::default_control_icons),
        badge_save: s.badge_save.clone().unwrap_or_else(|| badges.save.to_string()),
        badge_hint: s.badge_hint.clone().unwrap_or_else(|| badges.hint.to_string()),
        badge_hint_available: s.badge_hint_available.clone().unwrap_or_else(|| badges.hint_available.to_string()),
        diagonal_corners: s.diagonal_corners.unwrap_or_else(crate::config::default_diagonal_corners),
        overrides: s.overrides.clone(),
    }
}

// ── StyleColors ───────────────────────────────────────────────────────────────

/// Partial color configuration from a style file.
///
/// `scheme` is the optional named color scheme (e.g. `"tomorrow-night"`).
/// `selectors` maps CSS-ish selector names to their [`Decl`] blocks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleColors {
    pub scheme: Option<String>,
    pub selectors: BTreeMap<String, Decl>,
}

impl<'de> serde::Deserialize<'de> for StyleColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // The `[colors]` section is a flat map: `scheme` is a string and every
        // other key is a selector whose value is a [`Decl`] inline table. We
        // deserialize into a tolerant intermediate that accepts either shape per
        // key, mirroring `parse_style_toml`.
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum SchemeOrDecl {
            Scheme(String),
            Decl(Box<Decl>),
        }

        let raw: BTreeMap<String, SchemeOrDecl> = BTreeMap::deserialize(deserializer)?;
        let mut out = StyleColors::default();
        for (key, val) in raw {
            if key == "scheme" {
                if let SchemeOrDecl::Scheme(s) = val {
                    out.scheme = Some(s);
                }
            } else if let SchemeOrDecl::Decl(d) = val {
                out.selectors.insert(key, *d);
            }
            // Unknown shapes (e.g. non-string scheme) are ignored, never fatal.
        }
        Ok(out)
    }
}

// ── StyleDoc ──────────────────────────────────────────────────────────────────

/// A complete (but partial/raw) style document combining color and symbol config.
///
/// Every field uses `Option` or `BTreeMap` so absent fields are distinguished
/// from explicitly set ones. [`merge`] combines two `StyleDoc`s with
/// present-keys-only semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleDoc {
    pub colors: StyleColors,
    pub symbols: StyleSymbols,
    /// User story-styling rules from `[[transcript.rule]]`, in file order.
    pub transcript_rules: Vec<RawRule>,
    /// The status-bar block from `[statusbar]` / `[[statusbar.segment]]`.
    pub status_bar: RawStatusBar,
}

/// A raw (uncompiled) user transcript-styling rule from `[[transcript.rule]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawRule {
    /// The regex source string (from the rule's `match` key).
    pub pattern: String,
    /// The fg/bg/bold/italic style fields applied on a match.
    pub decl: Decl,
}

/// A raw (uncompiled) status-bar segment from `[[statusbar.segment]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSegment {
    /// Text template (literal text mixed with `{placeholder}` tokens).
    pub text: String,
    /// Cluster name: `left` | `center` | `right` (unknown → `left` at resolve).
    pub align: String,
    /// The fg/bg/bold/italic style fields for this segment.
    pub decl: Decl,
}

/// A raw `[statusbar]` block: optional frame + ordered segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawStatusBar {
    pub border: Option<String>,
    pub border_fg: Option<String>,
    pub segments: Vec<RawSegment>,
}

// ── merge ─────────────────────────────────────────────────────────────────────

/// Merge two [`StyleDoc`]s with present-keys-only semantics.
///
/// - `colors.scheme`: `over` wins if set, otherwise `base`.
/// - `colors.selectors`: union of keys; for a key in both, the `over` [`Decl`]
///   is field-merged onto the `base` [`Decl`] (each `Option` field: `over.or(base)`).
/// - `symbols` presets: `over.or(base)` per field.
/// - `symbols.overrides`: union of keys, `over` wins per key.
pub fn merge(base: &StyleDoc, over: &StyleDoc) -> StyleDoc {
    // colors.scheme
    let scheme = over.colors.scheme.clone().or(base.colors.scheme.clone());

    // colors.selectors: base ∪ over, with field-level merge for shared keys
    let mut selectors = base.colors.selectors.clone();
    for (key, over_decl) in &over.colors.selectors {
        let merged = if let Some(base_decl) = selectors.get(key) {
            merge_decl(base_decl, over_decl)
        } else {
            over_decl.clone()
        };
        selectors.insert(key.clone(), merged);
    }

    // symbols presets: over wins if set
    let symbols = StyleSymbols {
        box_style: over.symbols.box_style.clone().or(base.symbols.box_style.clone()),
        arrow_set: over.symbols.arrow_set.clone().or(base.symbols.arrow_set.clone()),
        portal_icons: over.symbols.portal_icons.clone().or(base.symbols.portal_icons.clone()),
        path_style: over.symbols.path_style.clone().or(base.symbols.path_style.clone()),
        portal_path_style: over.symbols.portal_path_style.clone().or(base.symbols.portal_path_style.clone()),
        control_icons: over.symbols.control_icons.clone().or(base.symbols.control_icons.clone()),
        badge_icons: over.symbols.badge_icons.clone().or(base.symbols.badge_icons.clone()),
        badge_save: over.symbols.badge_save.clone().or(base.symbols.badge_save.clone()),
        badge_hint: over.symbols.badge_hint.clone().or(base.symbols.badge_hint.clone()),
        badge_hint_available: over.symbols.badge_hint_available.clone().or(base.symbols.badge_hint_available.clone()),
        diagonal_corners: over.symbols.diagonal_corners.or(base.symbols.diagonal_corners),
        overrides: {
            let mut ov = base.symbols.overrides.clone();
            ov.extend(over.symbols.overrides.clone());
            ov
        },
    };

    let transcript_rules = if over.transcript_rules.is_empty() {
        base.transcript_rules.clone()
    } else {
        over.transcript_rules.clone()
    };

    let status_bar = RawStatusBar {
        border: over.status_bar.border.clone().or(base.status_bar.border.clone()),
        border_fg: over.status_bar.border_fg.clone().or(base.status_bar.border_fg.clone()),
        segments: if over.status_bar.segments.is_empty() {
            base.status_bar.segments.clone()
        } else {
            over.status_bar.segments.clone()
        },
    };

    StyleDoc {
        colors: StyleColors { scheme, selectors },
        symbols,
        transcript_rules,
        status_bar,
    }
}

/// Field-level merge of two [`Decl`]s: for each `Option` field, `over` wins if set.
fn merge_decl(base: &Decl, over: &Decl) -> Decl {
    Decl {
        fg:        over.fg.clone().or(base.fg.clone()),
        bg:        over.bg.clone().or(base.bg.clone()),
        bold:      over.bold.or(base.bold),
        italic:    over.italic.or(base.italic),
        underline: over.underline.or(base.underline),
        dim:       over.dim.or(base.dim),
        reversed:  over.reversed.or(base.reversed),
        style:     over.style.clone().or(base.style.clone()),
        style_top:    over.style_top.clone().or(base.style_top.clone()),
        style_bottom: over.style_bottom.clone().or(base.style_bottom.clone()),
        style_left:   over.style_left.clone().or(base.style_left.clone()),
        style_right:  over.style_right.clone().or(base.style_right.clone()),
        header:       over.header.or(base.header),
        shadow:    over.shadow.or(base.shadow),
        placement: over.placement.clone().or(base.placement.clone()),
        margin:    over.margin.or(base.margin),
        glyph_top:    over.glyph_top.clone().or(base.glyph_top.clone()),
        glyph_bottom: over.glyph_bottom.clone().or(base.glyph_bottom.clone()),
        glyph_left:   over.glyph_left.clone().or(base.glyph_left.clone()),
        glyph_right:  over.glyph_right.clone().or(base.glyph_right.clone()),
        glyph_tl:     over.glyph_tl.clone().or(base.glyph_tl.clone()),
        glyph_tr:     over.glyph_tr.clone().or(base.glyph_tr.clone()),
        glyph_bl:     over.glyph_bl.clone().or(base.glyph_bl.clone()),
        glyph_br:     over.glyph_br.clone().or(base.glyph_br.clone()),
    }
}

// ── parse_style_toml ─────────────────────────────────────────────────────────

/// Parse a style document from TOML text.
///
/// Accepts the format used by BOTH style files and `config.toml` override sections:
/// - `[colors]` with optional `scheme` string and selector keys as inline tables
///   (e.g. `"room:current" = { reversed = true }`).
/// - `[map]` with the glyph-set preset keys (`box_style`, `arrow_set`,
///   `portal_icons`, `path_style`, `portal_path_style`), the `diagonal_corners`
///   flag, and a `[map.overrides]` per-slot glyph table. `[map]`'s remaining
///   keys are colour selectors, read by [`theme::toml_schema`](crate::theme::toml_schema),
///   and ignored here.
/// - `[elements]` with the six story-picker `badge_*` glyph keys, which sit
///   beside the `story_badge` selector that colours them. As with `[map]`, the
///   section's remaining keys are colour selectors and are ignored here.
///
/// Unknown keys are ignored. Returns `Err(msg)` on TOML parse failure.
pub fn parse_style_toml(text: &str) -> Result<StyleDoc, String> {
    let root: toml::Value = text.parse().map_err(|e| format!("TOML parse error: {e}"))?;

    let mut colors = StyleColors::default();
    let mut symbols = StyleSymbols::default();

    if let Some(toml::Value::Table(colors_table)) = root.get("colors") {
        for (key, val) in colors_table {
            if key == "scheme" {
                if let Some(s) = val.as_str() {
                    colors.scheme = Some(s.to_string());
                }
            } else if let toml::Value::Table(decl_table) = val {
                // Each non-scheme key whose value is a table is a selector decl.
                let decl = parse_decl_from_table(decl_table);
                colors.selectors.insert(key.clone(), decl);
            }
            // Non-table, non-scheme keys are ignored (forward-compat).
        }
    }

    // The glyph-set presets live in `[map]` alongside the map's colour selectors
    // (the registry's `map.*` rows). Colour selectors are inline TABLES and are
    // resolved by `theme::toml_schema`; only the scalar preset keys and the
    // `overrides` table concern us here.
    if let Some(toml::Value::Table(map_table)) = root.get("map") {
        for (key, val) in map_table {
            match key.as_str() {
                "box_style"    => symbols.box_style    = val.as_str().map(str::to_string),
                "arrow_set"    => symbols.arrow_set    = val.as_str().map(str::to_string),
                "portal_icons" => symbols.portal_icons = val.as_str().map(str::to_string),
                "path_style"   => symbols.path_style   = val.as_str().map(str::to_string),
                "portal_path_style" => symbols.portal_path_style = val.as_str().map(str::to_string),
                "control_icons" => symbols.control_icons = val.as_str().map(str::to_string),
                "diagonal_corners"  => symbols.diagonal_corners  = val.as_bool(),
                "overrides" => {
                    if let toml::Value::Table(ov) = val {
                        for (ok, ov_val) in ov {
                            if let Some(s) = ov_val.as_str() {
                                symbols.overrides.insert(ok.clone(), s.to_string());
                            }
                        }
                    }
                }
                _ => {} // colour selectors + unknown keys: not symbol config
            }
        }
    }

    // The story picker's row badges live in `[elements]`, next to the
    // `story_badge` selector that colours them (SQ-0559) — one concept, one
    // place. Everything else under the header is a colour selector (an inline
    // TABLE) resolved by `theme::toml_schema`.
    if let Some(toml::Value::Table(el_table)) = root.get("elements") {
        for (key, val) in el_table {
            match key.as_str() {
                "badge_icons" => symbols.badge_icons = val.as_str().map(str::to_string),
                "badge_save"  => symbols.badge_save  = val.as_str().map(str::to_string),
                "badge_hint"  => symbols.badge_hint  = val.as_str().map(str::to_string),
                "badge_hint_available" => {
                    symbols.badge_hint_available = val.as_str().map(str::to_string)
                }
                _ => {} // colour selectors + unknown keys: not symbol config
            }
        }
    }

    let mut transcript_rules: Vec<RawRule> = Vec::new();
    if let Some(toml::Value::Table(tr_table)) = root.get("transcript") {
        if let Some(toml::Value::Array(rules)) = tr_table.get("rule") {
            for item in rules {
                if let toml::Value::Table(rt) = item {
                    let pattern = rt
                        .get("match")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if pattern.is_empty() {
                        continue; // a rule with no `match` is skipped
                    }
                    let decl = parse_decl_from_table(rt);
                    transcript_rules.push(RawRule { pattern, decl });
                }
            }
        }
    }

    let mut status_bar = RawStatusBar::default();
    if let Some(toml::Value::Table(sb)) = root.get("statusbar") {
        status_bar.border = sb.get("border").and_then(toml::Value::as_str).map(str::to_string);
        status_bar.border_fg = sb.get("border_fg").and_then(toml::Value::as_str).map(str::to_string);
        if let Some(toml::Value::Array(segs)) = sb.get("segment") {
            for item in segs {
                if let toml::Value::Table(st) = item {
                    let text = st.get("text").and_then(toml::Value::as_str).unwrap_or("").to_string();
                    let align = st.get("align").and_then(toml::Value::as_str).unwrap_or("left").to_string();
                    let decl = parse_decl_from_table(st);
                    status_bar.segments.push(RawSegment { text, align, decl });
                }
            }
        }
    }

    Ok(StyleDoc { colors, symbols, transcript_rules, status_bar })
}

/// Parse a [`Decl`] from a TOML inline table (field-by-field).
fn parse_decl_from_table(t: &toml::value::Table) -> Decl {
    Decl {
        fg:        t.get("fg").and_then(toml::Value::as_str).map(str::to_string),
        bg:        t.get("bg").and_then(toml::Value::as_str).map(str::to_string),
        bold:      t.get("bold").and_then(toml::Value::as_bool),
        italic:    t.get("italic").and_then(toml::Value::as_bool),
        underline: t.get("underline").and_then(toml::Value::as_bool),
        dim:       t.get("dim").and_then(toml::Value::as_bool),
        reversed:  t.get("reversed").and_then(toml::Value::as_bool),
        style:     t.get("style").and_then(toml::Value::as_str).map(str::to_string),
        style_top:    t.get("style_top").and_then(toml::Value::as_str).map(str::to_string),
        style_bottom: t.get("style_bottom").and_then(toml::Value::as_str).map(str::to_string),
        style_left:   t.get("style_left").and_then(toml::Value::as_str).map(str::to_string),
        style_right:  t.get("style_right").and_then(toml::Value::as_str).map(str::to_string),
        header:       t.get("header").and_then(toml::Value::as_bool),
        shadow:    t.get("shadow").and_then(toml::Value::as_bool),
        placement: t.get("placement").and_then(toml::Value::as_str).map(str::to_string),
        margin:    t.get("margin").and_then(toml::Value::as_integer).map(|n| n as u16),
        glyph_top:    t.get("glyph_top").and_then(toml::Value::as_str).map(str::to_string),
        glyph_bottom: t.get("glyph_bottom").and_then(toml::Value::as_str).map(str::to_string),
        glyph_left:   t.get("glyph_left").and_then(toml::Value::as_str).map(str::to_string),
        glyph_right:  t.get("glyph_right").and_then(toml::Value::as_str).map(str::to_string),
        glyph_tl:     t.get("glyph_tl").and_then(toml::Value::as_str).map(str::to_string),
        glyph_tr:     t.get("glyph_tr").and_then(toml::Value::as_str).map(str::to_string),
        glyph_bl:     t.get("glyph_bl").and_then(toml::Value::as_str).map(str::to_string),
        glyph_br:     t.get("glyph_br").and_then(toml::Value::as_str).map(str::to_string),
    }
}

// ── structural selector application ──────────────────────────────────────────

/// Resolve a border selector's per-side styles: each `style_<side>` override
/// parses on its own; an unset side falls back to the selector's base style.
fn resolve_sides(base: paneframe::BorderStyle, decl: &Decl) -> paneframe::PaneSides {
    let side = |ov: &Option<String>| -> paneframe::BorderStyle {
        match ov {
            None => base,
            Some(s) => paneframe::parse_border_style(s),
        }
    };
    paneframe::PaneSides {
        top: side(&decl.style_top),
        bottom: side(&decl.style_bottom),
        left: side(&decl.style_left),
        right: side(&decl.style_right),
    }
}

/// Collect a border selector's per-side/corner glyph overrides.
fn decl_glyphs(decl: &Decl) -> paneframe::PaneGlyphs {
    paneframe::PaneGlyphs {
        top:    decl.glyph_top.clone(),
        bottom: decl.glyph_bottom.clone(),
        left:   decl.glyph_left.clone(),
        right:  decl.glyph_right.clone(),
        tl:     decl.glyph_tl.clone(),
        tr:     decl.glyph_tr.clone(),
        bl:     decl.glyph_bl.clone(),
        br:     decl.glyph_br.clone(),
    }
}

/// Apply the STRUCTURAL channels of the `[colors]` selector decls onto the
/// `ColorScheme` (SQ-0641). Colour channels (fg/bg/modifiers) ride the registry
/// theme since SQ-0309 and are NOT applied here; but the structural fields —
/// border style + per-side styles, per-side/corner glyphs, the pane header
/// toggles, and the dialog's shadow/placement/margin — still live on
/// `ColorScheme` and had lost their only writer when `apply_color_decls` was
/// deleted, so `"dialog" = { shadow = true }` parsed, merged, round-tripped and
/// did nothing. Unknown selectors are ignored (they are colour selectors, or
/// forward-compat).
fn apply_structural_decls(cs: &mut ColorScheme, selectors: &BTreeMap<String, Decl>) {
    for (selector, decl) in selectors {
        match selector.as_str() {
            "map_border" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.map_border_style);
                cs.map_border_style = base;
                cs.map_border_sides = resolve_sides(base, decl);
                cs.map_border_glyphs = decl_glyphs(decl);
                if let Some(h) = decl.header {
                    cs.map_header_on = h;
                }
            }
            "story_border" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.story_border_style);
                cs.story_border_style = base;
                cs.story_border_sides = resolve_sides(base, decl);
                cs.story_border_glyphs = decl_glyphs(decl);
                if let Some(h) = decl.header {
                    cs.story_header_on = h;
                }
            }
            "status_header" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.status_header_style);
                cs.status_header_style = base;
                cs.status_header_sides = resolve_sides(base, decl);
                cs.status_header_glyphs = decl_glyphs(decl);
            }
            "input_line" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.input_line_style);
                cs.input_line_style = base;
                cs.input_line_sides = resolve_sides(base, decl);
                cs.input_line_glyphs = decl_glyphs(decl);
            }
            "suggestion_line" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.suggestion_line_style);
                cs.suggestion_line_style = base;
                cs.suggestion_line_sides = resolve_sides(base, decl);
                cs.suggestion_line_glyphs = decl_glyphs(decl);
            }
            "notification" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.notification_style);
                cs.notification_style = base;
                cs.notification_sides = resolve_sides(base, decl);
                cs.notification_glyphs = decl_glyphs(decl);
            }
            "upper_window_border" => {
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.virtual_window_border);
                cs.virtual_window_border = base;
                cs.upper_window_border_sides = resolve_sides(base, decl);
                cs.upper_window_border_glyphs = decl_glyphs(decl);
            }
            "dialog" => {
                if let Some(ref s) = decl.style {
                    cs.dialog_box_style = paneframe::parse_border_style(s);
                }
                if let Some(shadow_on) = decl.shadow {
                    cs.dialog_shadow_on = shadow_on;
                }
                if let Some(ref p) = decl.placement {
                    cs.dialog_placement = crate::render::dialog::DialogPlacement::from_token(p);
                }
                if let Some(m) = decl.margin {
                    cs.dialog_margin = m;
                }
                cs.dialog_glyphs = decl_glyphs(decl);
            }
            _ => {} // colour selectors (theme-resolved) and unknown names
        }
    }
}

/// Lower ONE selector's STRUCTURAL border channels — the `style` /
/// `style_<side>` keys, as the registry theme resolved them — onto the
/// `(base style, per-side styles)` pair the renderer actually frames with.
///
/// Precedence matches [`resolve_sides`]: a named side wins, an unnamed one takes
/// the selector's base `style`, and with no base at all the current value stands.
/// A selector the theme says nothing structural about leaves both outputs
/// untouched, so the legacy `[colors]` spelling — and the built-in default —
/// still stand.
fn lower_border(
    r: &crate::theme::resolve::Resolved,
    base_out: &mut paneframe::BorderStyle,
    sides_out: &mut paneframe::PaneSides,
) {
    let per_side = [r.border_top, r.border_bottom, r.border_left, r.border_right];
    if r.border.is_none() && per_side.iter().all(Option::is_none) {
        return;
    }
    if let Some(base) = r.border {
        *base_out = base;
    }
    let legacy = *sides_out;
    *sides_out = paneframe::PaneSides {
        top: r.border_top.or(r.border).unwrap_or(legacy.top),
        bottom: r.border_bottom.or(r.border).unwrap_or(legacy.bottom),
        left: r.border_left.or(r.border).unwrap_or(legacy.left),
        right: r.border_right.or(r.border).unwrap_or(legacy.right),
    };
}

/// Lower every frameable `[elements]` selector's structural channels back onto
/// the [`ColorScheme`] fields the renderers read (SQ-0700, SQ-0703).
///
/// Those fields had exactly one writer, [`apply_structural_decls`], which reads
/// the LEGACY `[colors]` selector table. The shipped `style.toml` (the
/// registry-generated template `style.example.toml` mirrors) has no `[colors]`
/// section at all — `upper_window_border`, `input_line` and `suggestion_line`
/// all live in `[elements]` — so a `style = "single"` parsed into the theme and
/// stopped there. Every one of those spellings is documented in the template
/// comments, and none of them worked from the file every user is handed.
///
/// Call this after the theme is rebuilt, so the documented spelling reaches
/// [`crate::render::upper_window::resolved_grid_sides`],
/// [`crate::render::screen::story_screen_dims`] and
/// [`crate::render::transcript::render_transcript`] alike — the frames stop (or
/// start) being drawn AND reserving the rows they are drawn in.
pub fn lower_element_borders(cs: &mut ColorScheme) {
    // `Theme::get` returns by value, so resolve all three first and the theme
    // borrow is over before the `&mut` field borrows begin.
    let upper = cs.theme.get("upper_window_border");
    let input = cs.theme.get("input_line");
    let suggestion = cs.theme.get("suggestion_line");
    lower_border(&upper, &mut cs.virtual_window_border, &mut cs.upper_window_border_sides);
    lower_border(&input, &mut cs.input_line_style, &mut cs.input_line_sides);
    lower_border(&suggestion, &mut cs.suggestion_line_style, &mut cs.suggestion_line_sides);
}

// ── resolve ───────────────────────────────────────────────────────────────────

/// Resolve a [`StyleDoc`] into a concrete [`ColorScheme`], [`SymbolSet`](crate::symbols::SymbolSet), and warnings.
///
/// Resolution:
/// 1. Build the base `ColorScheme` from `doc.colors.scheme` via `colors::resolve_base`
///    (handles `None` → terminal-default, built-in name, or file path).
/// 2. Obtain the active `GhosttyScheme` returned by `resolve_base` (or
///    `GhosttyScheme::default()` for the terminal-default case).
/// 3. Apply the structural channels of `doc.colors.selectors` (border styles,
///    sides, glyphs, headers, dialog shadow/placement/margin) — colour channels
///    ride the registry theme instead (SQ-0309/SQ-0641).
/// 4. Resolve symbols via `SymbolSet::resolve(&finalize_symbols(&doc.symbols))`.
///
/// Returns all warnings: base-scheme path/parse warnings.
pub fn resolve(
    doc: &StyleDoc,
    dir: &std::path::Path,
) -> (ColorScheme, crate::symbols::SymbolSet, Vec<String>) {
    // Step 1+2: build base ColorScheme and get the active GhosttyScheme.
    let (mut cs, gs, mut warnings) =
        colors::resolve_base(doc.colors.scheme.as_deref(), dir);

    // Step 3: structural selector channels (SQ-0641).
    apply_structural_decls(&mut cs, &doc.colors.selectors);

    // Compile user transcript rules; an invalid regex warns and is skipped.
    for r in &doc.transcript_rules {
        match regex::Regex::new(&r.pattern) {
            Ok(rx) => cs.transcript_rules.push(crate::colors::CompiledRule {
                pattern: r.pattern.clone(),
                regex: rx,
                style: decl_to_style(&r.decl, &gs),
            }),
            Err(e) => warnings.push(format!("invalid transcript rule regex '{}': {}", r.pattern, e)),
        }
    }

    // Compile the [statusbar] block. Segments replace the default layout only when
    // present; an empty block keeps the built-in default (today's bar).
    if !doc.status_bar.segments.is_empty() {
        let mut segments = Vec::with_capacity(doc.status_bar.segments.len());
        for raw in &doc.status_bar.segments {
            let align = match raw.align.as_str() {
                "left" => crate::colors::Align::Left,
                "center" => crate::colors::Align::Center,
                "right" => crate::colors::Align::Right,
                other => {
                    warnings.push(format!("unknown statusbar align '{}'; using left", other));
                    crate::colors::Align::Left
                }
            };
            segments.push(crate::colors::StatusSegment {
                text: raw.text.clone(),
                align,
                style: decl_to_style(&raw.decl, &gs),
            });
        }
        cs.statusbar_layout = crate::colors::StatusBarLayout { segments };
    }
    // The frame maps onto the existing status_header_style field (reuses the
    // boxing path). `border_fg` no longer has a legacy field to land on
    // (SQ-0309: `status_header`'s colour is theme-only now — see
    // `render/transcript.rs`'s `theme.get("status_header")`); it is parsed
    // above only for `write_style`'s round-trip.
    if let Some(b) = &doc.status_bar.border {
        cs.status_header_style = paneframe::parse_border_style(b);
    }

    // Step 4: resolve symbols.
    let set = crate::symbols::SymbolSet::resolve(&finalize_symbols(&doc.symbols));

    (cs, set, warnings)
}

// ── DEFAULT_STYLE_TOML ────────────────────────────────────────────────────────

/// The embedded built-in `default` style.
///
/// Sets single-line map and story borders as the default look.
/// An absent `[map]` means all glyph presets resolve to their factory defaults via finalize_symbols.
pub const DEFAULT_STYLE_TOML: &str = r#"# lanthorn built-in default style
# map_border / story_border = single; other selectors use terminal defaults.
# No [map] section: all glyph presets resolve to their factory defaults via finalize_symbols.

[colors]
"map_border" = { style = "single" }
"story_border" = { style = "single" }
"dialog" = { style = "single", bg = "black" }
"dialog:title" = { fg = "cyan" }
"dialog:button" = { fg = "white" }
"dialog:button:active" = { fg = "black", bg = "cyan" }
"dialog:shadow" = { bg = "dark-gray" }
"#;

// ── load_style ────────────────────────────────────────────────────────────────

/// Load a [`StyleDoc`] according to a pointer string.
///
/// Resolution order:
/// - `None` — if `user_dir/style.toml` exists, read and parse it; else parse
///   [`DEFAULT_STYLE_TOML`].
/// - `Some("default")` — always parse [`DEFAULT_STYLE_TOML`].
/// - `Some(path)` — `~`-expand and resolve relative to `user_dir`; read and parse
///   the file. On missing file or parse error, push exactly one warning string and
///   fall back to [`DEFAULT_STYLE_TOML`].
///
/// Never panics.
pub fn load_style(
    pointer: Option<&str>,
    user_dir: &std::path::Path,
) -> (StyleDoc, Vec<String>) {
    let default_doc = || parse_style_toml(DEFAULT_STYLE_TOML).expect("DEFAULT_STYLE_TOML must parse");

    match pointer {
        None => {
            let candidate = user_dir.join("style.toml");
            if candidate.is_file() {
                match std::fs::read_to_string(&candidate) {
                    Ok(text) => match parse_style_toml(&text) {
                        Ok(doc) => return (doc, Vec::new()),
                        Err(e) => {
                            let warn = format!(
                                "could not parse style file '{}': {}; using built-in default",
                                candidate.display(),
                                e
                            );
                            return (default_doc(), vec![warn]);
                        }
                    },
                    Err(e) => {
                        let warn = format!(
                            "could not read style file '{}': {}; using built-in default",
                            candidate.display(),
                            e
                        );
                        return (default_doc(), vec![warn]);
                    }
                }
            }
            (default_doc(), Vec::new())
        }
        Some("default") => (default_doc(), Vec::new()),
        Some(path_str) => {
            let path = colors::expand_path(path_str, user_dir);
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_style_toml(&text) {
                    Ok(doc) => (doc, Vec::new()),
                    Err(e) => {
                        let warn = format!(
                            "could not parse style file '{}': {}; using built-in default",
                            path.display(),
                            e
                        );
                        (default_doc(), vec![warn])
                    }
                },
                Err(e) => {
                    let warn = format!(
                        "could not read style file '{}': {}; using built-in default",
                        path.display(),
                        e
                    );
                    (default_doc(), vec![warn])
                }
            }
        }
    }
}

// ── personal_style_path ───────────────────────────────────────────────────────

/// The path to the user's personal style file: `user_dir/style.toml`.
///
/// This is the file written by gallery/config saves and the "Output all settings"
/// export; `config.style` is repointed at it so the saved look persists.
pub fn personal_style_path(user_dir: &std::path::Path) -> std::path::PathBuf {
    user_dir.join("style.toml")
}

// ── The font check's answer (SQ-1104) ─────────────────────────────────────────

/// The style file a setting should be WRITTEN to, resolved the same way
/// [`load_style`] resolves the one to read.
///
/// `None` when the pointer names the built-in `default` style, which lives in
/// the binary and has no file to write. An absent pointer means the personal
/// file, which is created if it is not there yet.
pub fn style_write_path(
    pointer: Option<&str>,
    user_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    match pointer {
        None => Some(personal_style_path(user_dir)),
        Some("default") => None,
        Some(path_str) => Some(colors::expand_path(path_str, user_dir)),
    }
}

/// Record the font check's answer(s) in `path`'s `[map]` and `[elements]`
/// sections as PRESET NAMES, format-preserving (SQ-1104, SQ-1159, SQ-1245).
///
/// Names, not expanded per-slot overrides. `arrow_set = "nerdfont"` is one line
/// a person can read and re-decide; the forty `[map.overrides]` entries it would
/// otherwise become freeze today's codepoints into the user's file and stop a
/// later improvement to the preset from ever reaching them.
///
/// The Guiding Light's mark is the exception, and only because it has no preset
/// of its own — it is a single glyph, so it is written as the single override it
/// is. A "plain" answer clears that override ONLY when it still holds the lamp
/// this function wrote; a mark the user chose themselves is not ours to remove.
///
/// `diagonal` is the SECOND question's answer (SQ-1245), independent of
/// `nerdfont` in both directions: `Some(true)`/`Some(false)` write
/// `map.diagonal_corners`, and `None` — the player skipped stage two — writes
/// nothing and leaves whatever the key already held untouched, exactly like an
/// absent key means "default" everywhere else in this file.
///
/// Refuses to touch a file that does not parse, for the same reason
/// [`crate::config::write_config_at`] does: a broken file is the text the user
/// needs to READ to fix it, and rewriting it destroys that.
pub fn write_font_check_answer(
    path: &std::path::Path,
    nerdfont: bool,
    diagonal: Option<bool>,
) -> std::io::Result<()> {
    use toml_edit::{DocumentMut, Item, value};

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = existing.parse().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{} is not valid TOML ({e}) — refusing to overwrite it. Fix the file, \
                 or move it aside and lanthorn will seed a fresh one.",
                path.display(),
            ),
        )
    })?;

    if !doc.contains_key("map") {
        doc["map"] = Item::Table(toml_edit::Table::new());
    }
    let lamp = crate::render::font_check_dialog::ASSIST_LAMP;
    let (arrows, portals, controls, badges) = if nerdfont {
        (
            crate::render::font_check_dialog::NERD_ARROWS,
            crate::render::font_check_dialog::NERD_PORTALS,
            crate::render::font_check_dialog::NERD_CONTROLS,
            crate::render::font_check_dialog::NERD_BADGES,
        )
    } else {
        ("filled", "ascii", "plain", "plain")
    };
    // Both answers are written out, not just the affirmative one: the file is
    // then a record of what was DECIDED, and a later re-check that swings the
    // other way lands on the same two keys instead of leaving a stale pair
    // behind (`/run-font-check` after changing terminal fonts is the whole point
    // of the re-check).
    let note = if nerdfont { "  # set by the font check (patched Nerd Font)" } else { "  # set by the font check" };
    for (key, name) in [("arrow_set", arrows), ("portal_icons", portals), ("control_icons", controls)] {
        doc["map"][key] = value(name);
        if let Some(v) = doc["map"][key].as_value_mut() {
            v.decor_mut().set_suffix(note);
        }
    }

    // The picker's row badges are the one preset that does NOT live in `[map]` —
    // they sit in `[elements]` beside the `story_badge` selector that colours
    // them (SQ-0559). One key, for the same reason the three above are one key
    // each: expanded per-badge glyphs would freeze today's codepoints into the
    // user's file (SQ-1159).
    if !doc.contains_key("elements") {
        doc["elements"] = Item::Table(toml_edit::Table::new());
    }
    doc["elements"]["badge_icons"] = value(badges);
    if let Some(v) = doc["elements"]["badge_icons"].as_value_mut() {
        v.decor_mut().set_suffix(note);
    }

    let map = doc["map"].as_table_mut().expect("[map] is a table");
    if nerdfont {
        if !map.contains_key("overrides") {
            map["overrides"] = Item::Table(toml_edit::Table::new());
        }
        if let Some(ov) = map["overrides"].as_table_mut() {
            ov["gutter.assist"] = value(lamp.to_string());
            if let Some(v) = ov["gutter.assist"].as_value_mut() {
                v.decor_mut().set_suffix("  # md-post_lamp U+F1A60 — the Guiding Light's mark");
            }
        }
    } else if let Some(ov) = map.get_mut("overrides").and_then(|o| o.as_table_mut()) {
        // Only OUR glyph is cleared. A mark the user picked by hand survives an
        // answer about a font, because it is not an answer about their mark.
        let ours = ov
            .get("gutter.assist")
            .and_then(|i| i.as_str())
            .is_some_and(|s| s.chars().eq(std::iter::once(lamp)));
        if ours {
            ov.remove("gutter.assist");
        }
    }

    // The diagonal answer (SQ-1245), independent of everything above it: a
    // bare bool, not a preset name, because `diagonal_corners` already is one —
    // see `crate::config::SymbolConfig`. `None` (stage two skipped) writes
    // nothing, leaving the key exactly as it was.
    if let Some(diag) = diagonal {
        let diag_note = if diag {
            "  # set by the font check (diagonal corner stubs)"
        } else {
            "  # set by the font check"
        };
        doc["map"]["diagonal_corners"] = value(diag);
        if let Some(v) = doc["map"]["diagonal_corners"].as_value_mut() {
            v.decor_mut().set_suffix(diag_note);
        }
    }

    std::fs::write(path, doc.to_string())
}

/// Encode a [`Color`] as a string suitable for a [`Decl`] fg/bg field.
///
/// `pub(crate)`: also reused by `theme::template::commented_template` (the
/// registry-driven `style.toml` template generator, SQ-0309) as the inverse of
/// `colors::parse_color_value`.
pub(crate) fn color_to_str(c: ratatui::style::Color) -> String {
    use ratatui::style::Color::*;
    match c {
        Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        Indexed(n) => n.to_string(),
        Black => "black".to_string(),
        Red => "red".to_string(),
        Green => "green".to_string(),
        Yellow => "yellow".to_string(),
        Blue => "blue".to_string(),
        Magenta => "magenta".to_string(),
        Cyan => "cyan".to_string(),
        Gray => "gray".to_string(),
        White => "white".to_string(),
        DarkGray => "dark-gray".to_string(),
        LightRed => "light-red".to_string(),
        LightGreen => "light-green".to_string(),
        LightYellow => "light-yellow".to_string(),
        LightBlue => "light-blue".to_string(),
        LightMagenta => "light-magenta".to_string(),
        LightCyan => "light-cyan".to_string(),
        Reset => "reset".to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;

    #[test]
    fn statusbar_block_parses_segments_and_border() {
        let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"

[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "cyan"
bold = true

[[statusbar.segment]]
text = "Score: {score}"
align = "right"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.status_bar.border.as_deref(), Some("single"));
        assert_eq!(doc.status_bar.border_fg.as_deref(), Some("cyan"));
        assert_eq!(doc.status_bar.segments.len(), 2);
        assert_eq!(doc.status_bar.segments[0].text, "{location}");
        assert_eq!(doc.status_bar.segments[0].align, "left");
        assert_eq!(doc.status_bar.segments[0].decl.fg.as_deref(), Some("cyan"));
        assert_eq!(doc.status_bar.segments[0].decl.bold, Some(true));
        assert_eq!(doc.status_bar.segments[1].align, "right");
    }

    #[test]
    fn style_example_toml_parses_and_resolves_clean() {
        // The repo-root style.example.toml is the user-facing reference (SQ-0309:
        // now the registry-generated, fully-commented new-schema template — see
        // `theme::template::style_example_matches_generated_template` for the
        // byte-for-byte check). It must still parse clean under the new schema so
        // the docs cannot drift from the code.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let parsed = crate::theme::toml_schema::parse(&text);
        assert!(parsed.is_ok(), "style.example.toml failed to parse: {parsed:?}");
    }

    #[test]
    fn resolve_statusbar_segments_border_and_align() {
        use crate::colors::Align;
        let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"
[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "yellow"
[[statusbar.segment]]
text = "{title}"
align = "center"
[[statusbar.segment]]
text = "{score}"
align = "bogus"
"##;
        let doc = parse_style_toml(text).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        // Three segments, with the unknown align defaulting to Left + a warning.
        assert_eq!(cs.statusbar_layout.segments.len(), 3);
        assert!(matches!(cs.statusbar_layout.segments[0].align, Align::Left));
        assert!(matches!(cs.statusbar_layout.segments[1].align, Align::Center));
        assert!(matches!(cs.statusbar_layout.segments[2].align, Align::Left));
        assert_eq!(cs.statusbar_layout.segments[0].style.fg, Some(ratatui::style::Color::Yellow));
        assert!(warnings.iter().any(|w| w.contains("align")), "unknown align warns: {warnings:?}");
        // border maps onto the existing status_header_style machinery. SQ-0309:
        // `border_fg` no longer lands on a legacy field (status_header's colour
        // is theme-only now) — parsing it still round-trips via `write_style`.
        assert!(matches!(cs.status_header_style, crate::render::paneframe::BorderStyle::Single));
    }

    #[test]
    fn resolve_no_statusbar_keeps_default_layout() {
        let (cs, _set, _w) = resolve(&StyleDoc::default(), std::path::Path::new("."));
        assert_eq!(cs.statusbar_layout, crate::colors::StatusBarLayout::default());
    }

    #[test]
    fn merge_replaces_statusbar_segments_when_override_has_any() {
        let mut base = StyleDoc::default();
        base.status_bar.segments.push(RawSegment { text: "a".into(), align: "left".into(), decl: Decl::default() });
        let mut over = StyleDoc::default();
        over.status_bar.segments.push(RawSegment { text: "b".into(), align: "right".into(), decl: Decl::default() });
        over.status_bar.border = Some("double".into());
        let m = merge(&base, &over);
        assert_eq!(m.status_bar.segments.len(), 1);
        assert_eq!(m.status_bar.segments[0].text, "b");
        assert_eq!(m.status_bar.border.as_deref(), Some("double"));
        // Empty override keeps base segments.
        let m2 = merge(&base, &StyleDoc::default());
        assert_eq!(m2.status_bar.segments[0].text, "a");
    }

    #[test]
    fn transcript_rules_parse_compile_in_order() {
        let text = r##"
[colors]
[[transcript.rule]]
match = "^>.*"
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"
fg = "red"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.transcript_rules.len(), 2);
        assert_eq!(doc.transcript_rules[0].pattern, "^>.*");
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(cs.transcript_rules.len(), 2);
        assert!(cs.transcript_rules[0].regex.is_match("> go north"));
        assert!(cs.transcript_rules[1].regex.is_match("A lurking GRUE!"));
        use ratatui::style::Color;
        assert_eq!(cs.transcript_rules[0].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn invalid_transcript_rule_warns_and_skips() {
        let text = r##"
[colors]
[[transcript.rule]]
match = "("
fg = "red"

[[transcript.rule]]
match = "ok"
fg = "green"
"##;
        let doc = parse_style_toml(text).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert_eq!(warnings.len(), 1, "exactly one invalid-regex warning: {warnings:?}");
        assert_eq!(cs.transcript_rules.len(), 1, "valid rule still loads");
        assert!(cs.transcript_rules[0].regex.is_match("ok"));
    }

    #[test]
    fn merge_replaces_transcript_rules_when_override_has_any() {
        let mut base = StyleDoc::default();
        base.transcript_rules.push(RawRule { pattern: "a".into(), decl: Decl::default() });
        let mut over = StyleDoc::default();
        over.transcript_rules.push(RawRule { pattern: "b".into(), decl: Decl::default() });
        let m = merge(&base, &over);
        assert_eq!(m.transcript_rules.len(), 1);
        assert_eq!(m.transcript_rules[0].pattern, "b");
        // Empty override keeps base rules.
        let m2 = merge(&base, &StyleDoc::default());
        assert_eq!(m2.transcript_rules[0].pattern, "a");
    }

    #[test]
    fn decl_to_style_sets_fg_and_modifiers() {
        use ratatui::style::{Color, Modifier};
        let scheme = crate::colors::GhosttyScheme::default();
        let d = Decl { fg: Some("cyan".into()), bold: Some(true), reversed: Some(true), ..Default::default() };
        let s = decl_to_style(&d, &scheme);
        assert_eq!(s.fg, Some(Color::Cyan));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert!(s.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(s.bg, None); // bg omitted => unset
    }

    #[test]
    fn decl_parses_placement_and_margin() {
        let t: toml::value::Table = toml::from_str(
            "placement = \"bottom\"\nmargin = 2\n",
        ).unwrap();
        let d = parse_decl_from_table(&t);
        assert_eq!(d.placement.as_deref(), Some("bottom"));
        assert_eq!(d.margin, Some(2));
        // Absent keys parse to None.
        let empty: toml::value::Table = toml::from_str("fg = \"cyan\"\n").unwrap();
        let d2 = parse_decl_from_table(&empty);
        assert_eq!(d2.placement, None);
        assert_eq!(d2.margin, None);
    }

    #[test]
    fn finalize_symbols_fills_defaults_and_keeps_overrides() {
        let mut s = StyleSymbols::default();
        s.box_style = Some("thick".into());
        s.overrides.insert("arrow.north".into(), "^".into());
        let cfg = finalize_symbols(&s);
        assert_eq!(cfg.box_style, "thick");
        assert_eq!(cfg.arrow_set, crate::config::default_arrow_set()); // unspecified => default
        assert_eq!(cfg.overrides.get("arrow.north").map(String::as_str), Some("^"));
        // resolve must succeed
        let _set = crate::symbols::SymbolSet::resolve(&cfg);
    }

    #[test]
    fn merge_override_only_affects_present_keys() {
        let mut base = StyleDoc::default();
        base.colors.selectors.insert("room".into(), Decl { fg: Some("white".into()), ..Default::default() });
        base.colors.selectors.insert("connector".into(), Decl { fg: Some("cyan".into()), ..Default::default() });
        base.symbols.box_style = Some("rounded".into());

        let mut over = StyleDoc::default();
        over.colors.selectors.insert("room".into(), Decl { fg: Some("red".into()), ..Default::default() });
        // over does not mention connector or box_style

        let m = merge(&base, &over);
        assert_eq!(m.colors.selectors["room"].fg.as_deref(), Some("red"));   // overridden
        assert_eq!(m.colors.selectors["connector"].fg.as_deref(), Some("cyan")); // base preserved
        assert_eq!(m.symbols.box_style.as_deref(), Some("rounded"));          // base preserved
    }

    #[test]
    fn merge_field_level_decl_patch() {
        let mut base = StyleDoc::default();
        base.colors.selectors.insert("room".into(), Decl { fg: Some("white".into()), bold: Some(true), ..Default::default() });
        let mut over = StyleDoc::default();
        over.colors.selectors.insert("room".into(), Decl { fg: Some("red".into()), ..Default::default() }); // only fg
        let m = merge(&base, &over);
        assert_eq!(m.colors.selectors["room"].fg.as_deref(), Some("red")); // over wins
        assert_eq!(m.colors.selectors["room"].bold, Some(true));            // base bold kept
    }

    #[test]
    fn resolve_empty_doc_equals_terminal_default() {
        let doc = StyleDoc::default();
        let (cs, set, _w) = resolve(&doc, std::path::Path::new("."));
        assert_eq!(cs, crate::colors::ColorScheme::terminal_default());
        assert_eq!(set, crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default()));
    }

    #[test]
    fn parse_style_toml_reads_selectors_scheme_symbols() {
        let text = r##"
[colors]
scheme = "tomorrow-night"
"room" = { fg = "white" }
"room:current" = { reversed = true }
"suggestion" = { fg = "#7a7a7a" }

[map]
box_style = "rounded"
room = { fg = "white" }
[map.overrides]
"arrow.north" = "^"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.colors.scheme.as_deref(), Some("tomorrow-night"));
        assert_eq!(doc.colors.selectors["room"].fg.as_deref(), Some("white"));
        assert_eq!(doc.colors.selectors["room:current"].reversed, Some(true));
        assert_eq!(doc.colors.selectors["suggestion"].fg.as_deref(), Some("#7a7a7a"));
        assert_eq!(doc.symbols.box_style.as_deref(), Some("rounded"));
        assert_eq!(doc.symbols.overrides["arrow.north"], "^");
    }

    #[test]
    fn load_style_default_name_parses_builtin() {
        let (doc, warns) = load_style(Some("default"), std::path::Path::new("/nonexistent"));
        assert!(warns.is_empty());
        let _ = doc; // parses without error
    }

    #[test]
    fn load_style_missing_path_warns_and_falls_back() {
        let (doc, warns) = load_style(Some("/no/such/style.toml"), std::path::Path::new("/tmp"));
        assert_eq!(warns.len(), 1);
        assert_eq!(doc, parse_style_toml(DEFAULT_STYLE_TOML).unwrap());
    }

    #[test]
    fn personal_style_path_is_user_dir_style_toml() {
        let p = personal_style_path(std::path::Path::new("/home/u/.lanthorn"));
        assert_eq!(p, std::path::Path::new("/home/u/.lanthorn/style.toml"));
    }

    // ── TOML → SymbolSet (SQ-0557 / SQ-0558) ─────────────────────────────────
    //
    // The tests above stop at `StyleDoc`, and `symbols.rs`'s stop at
    // `SymbolConfig` → `SymbolSet`. Nothing joined the two, which is exactly how
    // `diagonal_corners` shipped unparsed and how the presets came to be
    // documented under a section the parser never read. These walk the whole
    // path — style.toml TEXT → the `SymbolSet` the renderer draws with.

    /// Resolve a style.toml string straight to the renderer's `SymbolSet`.
    fn symbols_from_toml(text: &str) -> crate::symbols::SymbolSet {
        let doc = parse_style_toml(text).expect("style text must parse");
        crate::symbols::SymbolSet::resolve(&finalize_symbols(&doc.symbols))
    }

    #[test]
    fn map_diagonal_corners_false_reaches_the_symbol_set() {
        // SQ-0557: the escape hatch for fonts without Unicode 13 Legacy
        // Computing coverage. Before the `[map]` parser arm existed, the key was
        // dropped and the default `true` always won.
        assert!(
            crate::symbols::SymbolSet::default().diagonal_corners,
            "guard: the default is on, so `false` below is a real change"
        );
        let set = symbols_from_toml("[map]\ndiagonal_corners = false\n");
        assert!(!set.diagonal_corners, "[map] diagonal_corners = false must reach the renderer");
    }

    #[test]
    fn map_glyph_presets_reach_the_symbol_set() {
        // SQ-0558: every preset, set from the canonical `[map]` section, must
        // change the resolved glyphs.
        let set = symbols_from_toml(
            "[map]\n\
             box_style = \"double\"\n\
             arrow_set = \"line\"\n\
             portal_icons = \"nerdfont-stairs\"\n\
             path_style = \"heavy\"\n\
             portal_path_style = \"light\"\n",
        );
        let default = crate::symbols::SymbolSet::default();

        assert_eq!(set.room_normal, crate::symbols::BoxStyle::preset("double").unwrap());
        assert_ne!(set.room_normal, default.room_normal);
        assert_eq!(set.arrows, crate::symbols::Arrows::preset("line").unwrap());
        assert_ne!(set.arrows, default.arrows);
        assert_eq!(set.path, crate::symbols::PathGlyphs::preset("heavy").unwrap());
        assert_ne!(set.path, default.path);
        assert_eq!(set.portal.up, crate::symbols::PortalGlyphs::preset("nerdfont-stairs").unwrap().up);
        assert_ne!(set.portal.up, default.portal.up);
        // portal_path_style styles the up/down/in/out links on their own, so it
        // must win over the icon set's own ┊/┄ pair.
        assert_eq!((set.portal.path, set.portal.path_h), ('│', '─'));
        assert_ne!(set.portal.path, default.portal.path);
    }

    #[test]
    fn map_overrides_still_beat_the_preset() {
        let set = symbols_from_toml(
            "[map]\nbox_style = \"double\"\n[map.overrides]\n\"arrow.north\" = \"^\"\n",
        );
        assert_eq!(set.arrows.north, '^');
        assert_eq!(set.room_normal, crate::symbols::BoxStyle::preset("double").unwrap());
    }

    #[test]
    fn per_game_style_layers_over_the_global_presets() {
        // The `<game_dir>/style.toml` layering (`merge`) must carry the glyph
        // knobs too: the per-game file wins where it speaks, the global stands
        // where it is silent.
        let global = parse_style_toml(
            "[map]\nbox_style = \"double\"\npath_style = \"heavy\"\ndiagonal_corners = false\n",
        )
        .unwrap();
        let per_game = parse_style_toml("[map]\npath_style = \"dotted\"\n").unwrap();

        let set = crate::symbols::SymbolSet::resolve(&finalize_symbols(&merge(&global, &per_game).symbols));
        assert_eq!(set.path, crate::symbols::PathGlyphs::preset("dotted").unwrap()); // per-game wins
        assert_eq!(set.room_normal, crate::symbols::BoxStyle::preset("double").unwrap()); // global stands
        assert!(!set.diagonal_corners); // global stands
    }

    #[test]
    fn every_map_override_slot_the_example_file_documents_is_a_real_slot() {
        // SQ-0561: `[map.overrides]` is hand-written into the template rather than
        // generated from a registry row, so nothing structural stops it naming a
        // slot `apply_override` doesn't accept — which is exactly how SQ-0558's
        // documented-but-ignored keys shipped. Take every example key out of the
        // file we ship, override it with one distinctive narrow char, and require
        // the resolved SymbolSet to actually change. A typo'd slot is silently
        // dropped by `apply_override`, so it would leave the set untouched here.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path).expect("read style.example.toml");

        // Bounded to the block: skip to the header, then take only the commented
        // quoted-key lines directly under it. Without the `take_while` this runs on
        // to the end of the file and collects colour selectors from later sections.
        let keys: Vec<String> = text
            .lines()
            .skip_while(|l| !l.starts_with("[map.overrides]"))
            .skip(1)
            .take_while(|l| l.starts_with("# \""))
            .filter_map(|l| l.strip_prefix("# \""))
            .filter_map(|l| l.split('"').next())
            .map(str::to_string)
            .collect();
        assert!(!keys.is_empty(), "no documented [map.overrides] slots found");

        let baseline = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        for key in &keys {
            let mut cfg = crate::config::SymbolConfig::default();
            cfg.overrides.insert(key.clone(), "#".to_string());
            let set = crate::symbols::SymbolSet::resolve(&cfg);
            assert_ne!(
                set, baseline,
                "[map.overrides] documents slot {key:?}, but overriding it changed nothing — \
                 apply_override does not accept that key"
            );
        }
    }

    #[test]
    fn style_example_toml_presets_take_effect_when_uncommented() {
        // The shipped reference file can never again document a key under a
        // section the parser does not read: uncomment its own lines (with a
        // non-default value, so the effect is visible) and assert they land.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));

        // Rewrite `# key = <default>` in place as `key = <value>`.
        let uncomment = |text: &str, key: &str, value: &str| -> String {
            let mut hit = false;
            let out = text
                .lines()
                .map(|line| {
                    if line.starts_with(&format!("# {key} = ")) {
                        hit = true;
                        return format!("{key} = {value}");
                    }
                    line.to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(hit, "style.example.toml no longer documents a `{key}` line");
            out
        };

        let text = uncomment(&text, "box_style", "\"double\"");
        let text = uncomment(&text, "arrow_set", "\"line\"");
        let text = uncomment(&text, "path_style", "\"heavy\"");
        let text = uncomment(&text, "portal_path_style", "\"light\"");
        let text = uncomment(&text, "diagonal_corners", "false");
        // …and the `[elements]` badge glyphs (SQ-0559), which the same broken
        // link kept unreachable for even longer — they were never read from any
        // section at all.
        let text = uncomment(&text, "badge_save", "\"s\"");
        let text = uncomment(&text, "badge_hint", "\"?\"");
        let text = uncomment(&text, "badge_hint_available", "\"¿\"");

        let set = symbols_from_toml(&text);
        assert_eq!(set.room_normal, crate::symbols::BoxStyle::preset("double").unwrap());
        assert_eq!(set.arrows, crate::symbols::Arrows::preset("line").unwrap());
        assert_eq!(set.path, crate::symbols::PathGlyphs::preset("heavy").unwrap());
        assert_eq!((set.portal.path, set.portal.path_h), ('│', '─'));
        assert!(!set.diagonal_corners);

        let cfg = badges_from_toml(&text);
        assert_eq!(
            (cfg.badge_save.as_str(), cfg.badge_hint.as_str(), cfg.badge_hint_available.as_str()),
            ("s", "?", "¿"),
        );
    }

    // ── TOML → picker badge glyphs (SQ-0559) ─────────────────────────────────
    //
    // The `badge_*` fields have existed on `StyleSymbols`/`SymbolConfig` and fed
    // `picker::BadgeGlyphs` since the picker landed, but no parser arm ever read
    // them — not under `[symbols]`, not anywhere — so they were always the
    // built-in letters. They now live in `[elements]`, beside the `story_badge`
    // selector that colours them. Three of the original six were retired by
    // SQ-1160: SQ-0369 had moved the story type and the Blorb into the row's
    // TYPE column as text, so those keys reached no pixel.

    /// Resolve a style.toml string straight to the `SymbolConfig` the story
    /// picker reads its badge glyphs from.
    fn badges_from_toml(text: &str) -> crate::config::SymbolConfig {
        finalize_symbols(&parse_style_toml(text).expect("style text must parse").symbols)
    }

    #[test]
    fn elements_badge_glyphs_reach_the_symbol_config() {
        // Every badge gets its OWN value, so a parser arm that wired them all to
        // one key — or to each other — cannot pass.
        let cfg = badges_from_toml(
            "[elements]\n\
             badge_save = \"\u{f0c7}\"\n\
             badge_hint = \"\u{f059}\"\n\
             badge_hint_available = \"\u{f05a}\"\n",
        );
        assert_eq!(cfg.badge_save, "\u{f0c7}");
        assert_eq!(cfg.badge_hint, "\u{f059}");
        assert_eq!(cfg.badge_hint_available, "\u{f05a}");

        // …and through to the borrowed view the picker rows actually draw with.
        let glyphs = crate::picker::BadgeGlyphs::from_symbols(&cfg);
        assert_eq!(glyphs.save, "\u{f0c7}");
        assert_eq!(glyphs.hint_available, "\u{f05a}");
    }

    /// SQ-1160 retired `badge_zcode`, `badge_glulx` and `badge_blorb`. There is
    /// no shim and none is wanted — but a style.toml that still names one must
    /// LOAD, or retiring a key turns a working user file into a failing one.
    /// The `[elements]` walk drops an unrecognised scalar (`_ => {}`), and
    /// `theme::toml_schema` only reads inline TABLES from that section, so a
    /// retired key is ignored by both halves rather than warned about or
    /// refused.
    #[test]
    fn a_retired_badge_key_in_elements_is_ignored_not_an_error() {
        let doc = parse_style_toml(
            "[elements]\n\
             badge_zcode = \"Z\"\n\
             badge_glulx = \"G\"\n\
             badge_blorb = \"B\"\n\
             badge_save = \"★\"\n",
        )
        .expect("a retired badge key must not fail the style file");
        let cfg = finalize_symbols(&doc.symbols);
        assert_eq!(cfg.badge_save, "★", "the surviving key beside it still lands");
    }

    #[test]
    fn one_badge_set_leaves_the_others_at_their_defaults() {
        // The absent keys must still fall back to the built-in letters, and the
        // one that IS set must not bleed into its neighbours.
        let cfg = badges_from_toml("[elements]\nbadge_save = \"💾\"\n");
        assert_eq!(cfg.badge_save, "💾");
        assert_eq!(cfg.badge_hint, "H");
        assert_eq!(cfg.badge_hint_available, "h");
    }

    #[test]
    fn absent_elements_section_leaves_every_badge_at_its_default() {
        let cfg = badges_from_toml("[colors]\n\"transcript\" = { fg = \"red\" }\n");
        assert_eq!(
            (cfg.badge_save.as_str(), cfg.badge_hint.as_str(), cfg.badge_hint_available.as_str()),
            ("S", "H", "h"),
        );
    }

    // ── `badge_icons`, the badge PRESET (SQ-1159) ────────────────────────────
    //
    // Free-text keys and no preset behind them is what let the badges fall out
    // of step with the font check: `write_font_check_answer` set the arrows, the
    // portals and the border controls from one answer and could not reach these
    // at all. One name resolves the whole set; the keys stay as overrides.

    #[test]
    fn badge_icons_nerdfont_moves_every_badge_at_once() {
        let cfg = badges_from_toml("[elements]\nbadge_icons = \"nerdfont\"\n");
        let want = crate::symbols::StoryBadges::preset("nerdfont").unwrap();
        assert_eq!(
            (cfg.badge_save.as_str(), cfg.badge_hint.as_str(), cfg.badge_hint_available.as_str()),
            (
                want.save.to_string().as_str(),
                want.hint.to_string().as_str(),
                want.hint_available.to_string().as_str(),
            ),
        );
    }

    #[test]
    fn a_badge_key_set_by_hand_beats_the_preset_it_sits_under() {
        // The preset is the answer to "does my font draw icons"; a key is the
        // answer to "what do I want THIS badge to be". The second wins, and only
        // for its own slot.
        let cfg = badges_from_toml("[elements]\nbadge_icons = \"nerdfont\"\nbadge_save = \"★\"\n");
        let want = crate::symbols::StoryBadges::preset("nerdfont").unwrap();
        assert_eq!(cfg.badge_save, "★", "the hand-set key wins");
        assert_eq!(cfg.badge_hint, want.hint.to_string(), "its neighbours still come from the preset");
        assert_eq!(cfg.badge_hint_available, want.hint_available.to_string());
    }

    #[test]
    fn an_unknown_badge_preset_name_falls_back_to_the_letters() {
        // Same treatment every other category gives an unknown name: keep the
        // default rather than draw nothing.
        let cfg = badges_from_toml("[elements]\nbadge_icons = \"sparkles\"\n");
        assert_eq!(cfg.badge_save, "S");
        assert_eq!(cfg.badge_hint_available, "h");
    }

    #[test]
    fn per_game_style_can_swap_the_whole_badge_set() {
        let global = parse_style_toml("[elements]\nbadge_icons = \"nerdfont\"\n").unwrap();
        let per_game = parse_style_toml("[elements]\nbadge_icons = \"plain\"\n").unwrap();
        let cfg = finalize_symbols(&merge(&global, &per_game).symbols);
        assert_eq!(cfg.badge_save, "S", "the per-game preset wins outright");
    }

    #[test]
    fn per_game_style_layers_over_the_global_badges() {
        // `merge` already carried the badge fields; now that the parser fills
        // them, pin that the layering actually works end to end.
        let global =
            parse_style_toml("[elements]\nbadge_hint_available = \"a\"\nbadge_save = \"s\"\n").unwrap();
        let per_game = parse_style_toml("[elements]\nbadge_save = \"★\"\n").unwrap();
        let cfg = finalize_symbols(&merge(&global, &per_game).symbols);
        assert_eq!(cfg.badge_save, "★"); // per-game wins
        assert_eq!(cfg.badge_hint_available, "a"); // global stands
        assert_eq!(cfg.badge_hint, "H"); // neither spoke → default
    }

    #[test]
    fn element_colour_selectors_are_not_mistaken_for_badge_glyphs() {
        // `[elements]` is mostly colour selectors (inline tables). They must pass
        // straight through the badge arms without disturbing anything.
        let cfg = badges_from_toml(
            "[elements]\nstory_badge = { fg = \"cyan\" }\nbadge_hint = \"¡\"\n",
        );
        assert_eq!(cfg.badge_hint, "¡");
        assert_eq!(cfg.badge_save, "S");
    }

    #[test]
    fn resolve_sets_border_style_and_default_is_single() {
        // default doc (DEFAULT_STYLE_TOML) => single map, single story (SQ-0357)
        let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
        let (cs, _set, _w) = resolve(&doc, std::path::Path::new("."));
        assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::Single));
        assert!(matches!(cs.story_border_style, crate::render::paneframe::BorderStyle::Single));

        // SQ-0641: the SCHEME case must pin the same structure. This test used
        // to cover only the no-scheme doc, which is exactly how from_ghostty's
        // None seeding (scheme choice silently dropping the pane borders and
        // doubling the map layer strip) shipped unnoticed.
        let doc = parse_style_toml("[colors]\nscheme = \"tomorrow-night\"\n").unwrap();
        let (cs, _set, w) = resolve(&doc, std::path::Path::new("."));
        assert!(w.is_empty(), "built-in scheme resolves clean: {w:?}");
        assert!(
            matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::Single),
            "a colour scheme must not flip the map border to None"
        );
        assert!(matches!(cs.story_border_style, crate::render::paneframe::BorderStyle::Single));
        assert_eq!(
            cs.map_border_sides,
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single)
        );
    }

    // ── SQ-0641: [colors] structural selectors reach the ColorScheme ──────────
    //
    // `resolve` stopped reading `doc.colors.selectors` when apply_color_decls
    // was deleted (SQ-0309), so every structural field (dialog shadow/placement/
    // margin, pane headers, per-side border styles, border glyphs) parsed,
    // merged, round-tripped — and did nothing. These walk style.toml TEXT → the
    // exact ColorScheme field the render path consumes.

    /// Resolve a style.toml string straight to the ColorScheme.
    fn colors_from_toml(text: &str) -> crate::colors::ColorScheme {
        let doc = parse_style_toml(text).expect("style text must parse");
        resolve(&doc, std::path::Path::new(".")).0
    }

    #[test]
    fn dialog_shadow_placement_margin_reach_the_colorscheme() {
        let base = crate::colors::ColorScheme::terminal_default();
        assert!(!base.dialog_shadow_on, "guard: shadow defaults off, so true below is a change");
        let cs = colors_from_toml(
            "[colors]\n\"dialog\" = { shadow = true, placement = \"bottom\", margin = 2 }\n",
        );
        assert!(cs.dialog_shadow_on, "dialog shadow = true must reach dialog_shadow_on");
        assert_eq!(cs.dialog_placement, crate::render::dialog::DialogPlacement::Bottom);
        assert_eq!(cs.dialog_margin, 2);
        // The render seam consumes exactly these fields (DialogStyle::from_colors).
        let ds = crate::render::dialog::DialogStyle::from_colors(&cs);
        assert!(ds.shadow_on);
        assert_eq!(ds.placement, crate::render::dialog::DialogPlacement::Bottom);
        assert_eq!(ds.margin, 2);
    }

    #[test]
    fn dialog_border_style_and_glyphs_reach_the_colorscheme() {
        let cs = colors_from_toml(
            "[colors]\n\"dialog\" = { style = \"double\", glyph_tl = \"+\" }\n",
        );
        assert!(matches!(cs.dialog_box_style, crate::render::paneframe::BorderStyle::Double));
        assert_eq!(cs.dialog_glyphs.tl.as_deref(), Some("+"));
    }

    #[test]
    fn pane_header_toggles_reach_the_colorscheme() {
        let base = crate::colors::ColorScheme::terminal_default();
        assert!(base.map_header_on && base.story_header_on, "guard: headers default on");
        let cs = colors_from_toml(
            "[colors]\n\"map_border\" = { header = false }\n\"story_border\" = { header = false }\n",
        );
        assert!(!cs.map_header_on, "map_border header = false must reach map_header_on");
        assert!(!cs.story_header_on, "story_border header = false must reach story_header_on");
    }

    #[test]
    fn border_style_sides_and_glyphs_reach_the_colorscheme() {
        use crate::render::paneframe::BorderStyle;
        let cs = colors_from_toml(
            "[colors]\n\
             \"map_border\" = { style = \"double\", style_top = \"thick\", glyph_tl = \"◆\" }\n\
             \"story_border\" = { style_bottom = \"none\" }\n",
        );
        assert!(matches!(cs.map_border_style, BorderStyle::Double));
        assert!(matches!(cs.map_border_sides.top, BorderStyle::Thick), "per-side override wins");
        assert!(matches!(cs.map_border_sides.bottom, BorderStyle::Double), "unset side = base");
        assert_eq!(cs.map_border_glyphs.tl.as_deref(), Some("◆"));
        // story: no base style set → sides derive from the default Single base.
        assert!(matches!(cs.story_border_style, BorderStyle::Single));
        assert!(matches!(cs.story_border_sides.bottom, BorderStyle::None));
        assert!(matches!(cs.story_border_sides.top, BorderStyle::Single));
    }

    #[test]
    fn every_border_selector_family_member_reaches_its_fields() {
        use crate::render::paneframe::BorderStyle;
        let cs = colors_from_toml(
            "[colors]\n\
             \"status_header\" = { style = \"single\", glyph_left = \"|\" }\n\
             \"input_line\" = { style = \"single\" }\n\
             \"suggestion_line\" = { style = \"rounded\" }\n\
             \"notification\" = { style = \"double\" }\n\
             \"upper_window_border\" = { style = \"thick\", style_right = \"none\" }\n",
        );
        assert!(matches!(cs.status_header_style, BorderStyle::Single));
        assert_eq!(cs.status_header_glyphs.left.as_deref(), Some("|"));
        assert!(matches!(cs.input_line_style, BorderStyle::Single));
        assert_eq!(cs.input_line_sides, crate::render::paneframe::PaneSides::all(BorderStyle::Single));
        assert!(matches!(cs.suggestion_line_style, BorderStyle::Rounded));
        assert!(matches!(cs.notification_style, BorderStyle::Double));
        assert!(matches!(cs.virtual_window_border, BorderStyle::Thick));
        assert!(matches!(cs.upper_window_border_sides.right, BorderStyle::None));
        assert!(matches!(cs.upper_window_border_sides.left, BorderStyle::Thick));
    }

    #[test]
    fn per_game_style_layers_over_the_global_structural_decls() {
        // The `merge` layering must carry the structural channels end to end:
        // the per-game file wins where it speaks, the global stands elsewhere.
        let global = parse_style_toml(
            "[colors]\n\"dialog\" = { shadow = true, margin = 3 }\n\"map_border\" = { header = false }\n",
        )
        .unwrap();
        let per_game = parse_style_toml("[colors]\n\"dialog\" = { margin = 1 }\n").unwrap();
        let (cs, _set, _w) = resolve(&merge(&global, &per_game), std::path::Path::new("."));
        assert_eq!(cs.dialog_margin, 1, "per-game margin wins");
        assert!(cs.dialog_shadow_on, "global shadow stands");
        assert!(!cs.map_header_on, "global header toggle stands");
    }

}
