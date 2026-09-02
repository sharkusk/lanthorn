//! Registry-driven commented `style.toml` template (SQ-0309, Task 6b).
//!
//! [`commented_template`] emits the whole new-schema `style.toml`, grouped by
//! [`Section`] and with every line commented out — a no-op until the user
//! uncomments a line. Each selector's line is reconstructed straight from its
//! [`RegRow`]'s `parent` + `default_delta`, so uncommenting it reproduces
//! exactly the registry default (see `theme::resolve`). This replaces the
//! interactive style editors: [`auto_seed`] writes this template to the user's
//! `style.toml` on first run, and the file is then edited by hand + applied
//! live with `/reload`.

use super::registry::{Kind, RegRow, Section, DIAGONAL_CORNERS_DEFAULT, REGISTRY};
use crate::style::{color_to_str, personal_style_path};

/// Auto-seed `user_dir/style.toml` with [`commented_template`] if it does not
/// already exist. NEVER overwrites an existing file. Best-effort: a write
/// failure (e.g. a read-only home) is swallowed so startup never crashes.
pub fn auto_seed(user_dir: &std::path::Path) {
    let path = personal_style_path(user_dir);
    if path.exists() {
        return;
    }
    let _ = std::fs::write(&path, commented_template());
}

// A section to emit: (registry `Section`, TOML header, a short blurb comment
// shown above the header). Sections fall into two groups — TEXT (colour/emphasis
// only) and SURFACE (a background + optional border + the text drawn on them) —
// so it's clear where you adjust text vs. where you adjust a bordered box.

/// The roles: the roots everything else derives from, emitted first.
const ROLES_SECTION: (Section, &str, &str) =
    (Section::Roles, "[roles]", "Roles: the 7 roots everything else derives from.");

/// TEXT sections: adjust the foreground colour / emphasis of text, story output,
/// the map, and the debug view. These have no surface (background/border) of
/// their own — that's what the SURFACE sections below are for.
const TEXT_SECTIONS: &[(Section, &str, &str)] = &[
    (
        Section::Elements,
        "[elements]",
        "Elements: app text + host surfaces (status/help bars, upper window,\n# story browser). Every line already equals its default.",
    ),
    (Section::GlkBuffer, "[glk.buffer]", "The 11 Glk styles for text-buffer windows (base: text)."),
    (Section::GlkGrid, "[glk.grid]", "The 11 Glk styles for text-grid windows (base: chrome)."),
    (Section::Map, "[map]", "Map colours + glyph-set presets."),
    (Section::Debug, "[debug]", "Debug inspector disassembly selectors."),
];

/// SURFACE sections: bordered boxes with a background + frame + the text drawn
/// on them. Each is a distinct surface — a tiled pane, a modal, a tooltip.
const SURFACE_SECTIONS: &[(Section, &str, &str)] = &[
    (
        Section::Panel,
        "[panel]",
        "Panel: the tiled panes (story/map/verb/debug frames) — background,\n# border, title, tabs. Story/Glk windows use [glk.*], not this section.",
    ),
    (Section::Dialog, "[dialog]", "Dialog: modal pop-ups — background, its own border, title, buttons, shadow."),
    (Section::Tooltip, "[tooltip]", "Tooltip: hover pop-ups — background and an optional border."),
];

/// Emit the full new-schema `style.toml`, entirely commented out.
pub fn commented_template() -> String {
    let mut out = String::new();

    out.push_str(
        "# lanthorn style.toml — auto-seeded. The section headers below are active,\n\
         # but every value line is commented out, so the file stays a no-op until you\n\
         # uncomment lines: each commented value already equals the built-in default,\n\
         # reconstructed straight from the style registry. Edit, uncomment, save, then\n\
         # run reload-style in-app to apply changes live (or turn on config.toml's\n\
         # watch_style to auto-reload on save).\n\
         #\n\
         # Color values accept: a named color (cyan, dark-gray, light-blue, …),\n\
         # palette:N (0-15 from the active scheme), #rrggbb hex, a 256-index (\"17\"),\n\
         # or background / foreground (the scheme's bg/fg).\n\
         \n",
    );
    out.push_str(&format!(
        "version = {}   # style schema version — do not remove; lets lanthorn flag an out-of-date file\n\n",
        super::toml_schema::STYLE_SCHEMA_VERSION
    ));
    out.push_str(
        "# scheme = \"tomorrow-night\"   # optional base: built-in name or a Ghostty theme path; omit for terminal colours\n\
         \n",
    );

    let emit = |out: &mut String, section: Section, header: &str, blurb: &str| {
        out.push_str("# ── ");
        out.push_str(blurb);
        out.push('\n');
        out.push_str(header);
        out.push('\n');
        for row in REGISTRY.iter().filter(|r| r.section == section) {
            out.push_str(&row_line(section, row));
            out.push('\n');
        }
        // `[map.overrides]` is a free-form table keyed by glyph slot, not a
        // selector, so it has no registry row and must be appended by hand —
        // and it has to come AFTER every `[map]` key line, or TOML would bind
        // those keys to the sub-table instead (SQ-0561).
        if section == Section::Map {
            out.push_str(MAP_OVERRIDES_BLOCK);
        }
        out.push('\n');
    };

    // Roles first (the roots).
    emit(&mut out, ROLES_SECTION.0, ROLES_SECTION.1, ROLES_SECTION.2);

    // TEXT group.
    out.push_str("# ═══ TEXT — foreground colour + emphasis (these selectors have no surface of their own) ═══\n\n");
    for (section, header, blurb) in TEXT_SECTIONS {
        emit(&mut out, *section, header, blurb);
    }

    // SURFACE group.
    out.push_str("# ═══ SURFACES — bordered boxes: a background + frame + the text drawn on them ═══\n\n");
    for (section, header, blurb) in SURFACE_SECTIONS {
        emit(&mut out, *section, header, blurb);
    }

    out.push_str(STATIC_EXAMPLES);

    out
}

/// Reconstruct one registry row as a single commented TOML line.
fn row_line(section: Section, row: &RegRow) -> String {
    // Roles carry no registry delta (their concrete values live in
    // `Roles::from_scheme`), so emit each one's actual scheme-relative default
    // plus a one-line description of what it's for.
    if section == Section::Roles {
        return role_line(row.name);
    }

    // The one map row whose value `Delta` cannot carry: a bool.
    // (SQ-0641: `map.layer_cycle`, the other special case, was removed — the
    // list was documented in the template but had no consumer anywhere.)
    if row.name == "map.diagonal_corners" {
        return format!(
            "# diagonal_corners = {DIAGONAL_CORNERS_DEFAULT}   \
             # false = plain orthogonal corner exits, for fonts without Unicode 13 (🮠🮡🮢🮣)"
        );
    }

    let key = toml_key(&strip_section_prefix(section, row.name));

    if row.kind == Kind::Placement {
        let preset = row.default_delta.glyph.as_deref().unwrap_or_default();
        return format!("# {key} = \"{preset}\"{}", enum_hint(row));
    }

    let fields = row_fields(row);
    let line = if fields.is_empty() {
        format!("# {key} = {{}}")
    } else {
        format!("# {key} = {{ {} }}", fields.join(", "))
    };
    format!("{line}{}", enum_hint(row))
}

/// One commented `[roles]` line: the role's scheme-relative default (matching
/// `Roles::from_scheme`) plus a one-line description of its purpose. Kept in
/// sync with the resolver by `uncommented_template_resolves_to_registry_defaults`.
fn role_line(name: &str) -> String {
    let (value, desc) = match name {
        "text" => ("{ fg = \"foreground\" }", "body ink on the page (scheme foreground)"),
        "chrome" => ("{ fg = \"foreground\", bg = \"background\" }", "ink on a UI surface: bars, panels, upper window"),
        "line" => ("{ fg = \"palette:6\" }", "lines, frames, rules, dividers (scheme cyan slot)"),
        "accent" => ("{ fg = \"palette:6\" }", "highlights: links, selection, current room, badges"),
        "muted" => ("{ fg = \"palette:8\" }", "dim / secondary text (scheme bright-black slot)"),
        "alert" => ("{ fg = \"palette:3\" }", "warnings and errors (scheme yellow slot)"),
        "heading" => ("{ fg = \"foreground\", bold = true }", "titles and headers (bold)"),
        _ => return String::new(),
    };
    format!("# {name:<7} = {value:<40}  # {desc}")
}

/// The `# a | b | c` enumeration hint appended to an enumerated row's line
/// (a map glyph-set preset, or a border `style`), or `""` for a non-enumerated
/// row. Value lists are verified against `symbols.rs`'s preset tables and
/// `paneframe::parse_border_style`.
fn enum_hint(row: &RegRow) -> &'static str {
    match row.name {
        "map.box_style" => "   # rounded | thick | double | solid | super-thick | ascii | borderless",
        "map.arrow_set" => "   # filled | line | nerdfont | nf-bold | nf-box | nf-chevron | nf-circle | nf-outline",
        "map.portal_icons" => "   # ascii | nerdfont | nerdfont-stairs",
        "map.path_style" | "map.portal_path_style" => "   # light | heavy | dotted",
        "map.control_icons" => "   # plain | nerdfont   (every border control: both panes' clusters and the tooltip pointer)",
        // Story-list row badges (SQ-0559). `badge_icons` IS enumerated — it names
        // a `StoryBadges` preset, and the font check writes it (SQ-1159). The
        // three under it are free-form: any string, so a patched font can use a
        // real icon instead of a bare letter, and one set by hand beats the
        // preset.
        "badge_icons" => "   # plain | nerdfont   (the story-list badges below; each key overrides one)",
        "badge_save" => "   # story-list badge: the story has a save",
        "badge_hint" => "   # story-list badge: hints are installed",
        "badge_hint_available" => "   # story-list badge: hints exist but aren't downloaded",
        // SQ-1105: these three carry a `glyph` that MIRRORS the gutter default so
        // the two spellings sit near each other in the registry — but the renderer
        // draws `symbols.*_gutter`, not this. Without a note the template reads as
        // an invitation to set a character here and watch nothing happen, which is
        // exactly what the customization doc used to promise. Say where the mark
        // really comes from, at the one place the reader is holding the key.
        "transcript_meta" => "   # colour only — the gutter mark is [map.overrides] \"gutter.meta\"",
        "transcript_warning" => "   # colour only — the gutter mark is [map.overrides] \"gutter.warning\"",
        "transcript_assist" | "transcript_assist_caution" => {
            "   # colour only — the gutter mark is [map.overrides] \"gutter.assist\""
        }
        // SQ-0700: this one draws a FRAME as well as colouring it, but its default
        // delta carries no border (the frame's default lives on `ColorScheme`), so
        // the generic border hint below never fired and the `style` key that turns
        // the frame off was documented nowhere.
        "upper_window_border" => {
            "   # frame round a game's status/upper window; none by default — \
             style = \"single\" (or double | thick | rounded, or per-side \
             style_top/bottom/left/right) boxes it"
        }
        _ if row.default_delta.border.is_some() => "   # style: none | single | double | thick | rounded",
        _ => "",
    }
}

/// The `parent = ...` + delta fields for a row's inline table, in the order a
/// user would naturally write them.
fn row_fields(row: &RegRow) -> Vec<String> {
    let d = &row.default_delta;
    let mut fields = Vec::new();
    if let Some(parent) = row.parent {
        fields.push(format!("parent = \"{parent}\""));
    }
    if let Some(fg) = d.fg {
        fields.push(format!("fg = \"{}\"", color_to_str(fg)));
    }
    if let Some(bg) = d.bg {
        fields.push(format!("bg = \"{}\"", color_to_str(bg)));
    }
    // Tri-state (SQ-1171): an unset modifier is not mentioned, and BOTH `true` and
    // `false` are written out. `false` is a real default now — it removes a modifier
    // the parent set — so omitting it would document a row as inheriting a weight it
    // actually sheds, and uncommenting the line would then change the appearance
    // rather than restate it.
    for (flag, key) in [
        (d.bold, "bold"),
        (d.italic, "italic"),
        (d.underline, "underline"),
        (d.reversed, "reversed"),
        (d.dim, "dim"),
    ] {
        if let Some(v) = flag {
            fields.push(format!("{key} = {v}"));
        }
    }
    if let Some(border) = d.border {
        fields.push(format!("style = \"{}\"", crate::render::paneframe::border_style_name(border)));
    }
    if let Some(glyph) = &d.glyph {
        fields.push(format!("glyph = \"{glyph}\""));
    }
    fields
}

/// Strip a section's TOML-header prefix off a full registry selector name,
/// leaving the bare key written under that header (e.g. `panel.border:active`
/// → `border:active`, `glk.buffer.normal` → `normal`).
fn strip_section_prefix(section: Section, name: &str) -> String {
    let prefix = match section {
        Section::Panel => "panel.",
        Section::GlkBuffer => "glk.buffer.",
        Section::GlkGrid => "glk.grid.",
        Section::Map => "map.",
        Section::Debug => "debug.",
        Section::Dialog => "dialog.",
        Section::Tooltip => "tooltip.",
        Section::Roles | Section::Elements | Section::Statusbar => "",
    };
    name.strip_prefix(prefix).unwrap_or(name).to_string()
}

/// Quote a TOML key if it contains `:` or `.` (e.g. `"border:active"`).
fn toml_key(key: &str) -> String {
    if key.contains(':') || key.contains('.') {
        format!("\"{key}\"")
    } else {
        key.to_string()
    }
}

/// Static commented examples with no registry row of their own: a
/// `[[transcript.rule]]` pair and a `[statusbar]` + `[[statusbar.segment]]`
/// block, copied (commented) from the design spec's example.
/// The `[map.overrides]` table: swap ONE glyph without changing a whole preset.
///
/// Not a registry row — it is a free-form table keyed by glyph slot, so the
/// generic row path can't emit it and it is spelled out here (SQ-0561). Slot
/// names are validated against `symbols::apply_override`, which accepts 56 keys
/// across the five families listed below; an unknown key, or a value that isn't
/// exactly one narrow character, is ignored.
const MAP_OVERRIDES_BLOCK: &str = r#"
# ── One glyph at a time. An override beats the preset above it, so you can keep ─
# ── a whole preset and change only the one corner you dislike. Values must be ───
# ── exactly ONE narrow character; anything else is ignored. ─────────────────────
[map.overrides]
# "room.normal.tl" = "┌"     # room box corners/edges: room.<normal|current|selected|portal>.<tl|tr|bl|br|h|v>
# "arrow.north" = "▲"        # connector arrowheads: arrow.<north|south|east|west|ne|nw|se|sw>
# "path.ns" = "│"            # connector lines: path.<ns|ew|ne|nw|se|sw|nse|nsw|ews|ewn|cross>
# "path.diag_ul" = "🮠"       # the four half-diagonals: path.<diag_ul|diag_ur|diag_ll|diag_lr>
# "portal.up" = "↑"          # portal markers: portal.<up|down|in|out|marker|path|unknown>
# "gutter.meta" = "▏"        # transcript gutter marks: gutter.<meta|warning|assist>
# "gutter.assist" = "●"      # the mark of Lanthorn's Guiding Light (a patched font's lamp: U+F1A60)
# "control.map_hide" = "▶"   # border toggles: control.<map_show|map_hide|band_show|band_hide|inventory_open>
# "control.guidance_on" = "●" # …control.<guidance_on|guidance_off>
# "control.lock_on" = "▣"    # …control.<render_hybrid|render_raster|render_extended|lock_on|lock_off>
# "map_control.centre" = "¤" # map border cluster: map_control.<room_numbers_on|room_numbers_off|centre|zoom_out|zoom_in|view_matrix|view_drawn>
# "map_control.view_drawn" = "M" # the plain set repeats one mark per pair (# #, M M) and lets colour say which
#                            # state you are in; set the _off/_drawn half for a SHAPE change instead. On a
#                            # patched font that is md-numeric_off (U+F19D3) and md-grid_off (U+F02C2).
"#;

const STATIC_EXAMPLES: &str = r#"# ── Story-line styling rules: recolour whole transcript lines matching a ────
# ── regex. Rules are tried in order; the first match wins. ──────────────────
# [[transcript.rule]]
# match = "^>.*"                 # your echoed command lines → magenta bold
# fg = "magenta"
# bold = true

# [[transcript.rule]]
# match = "(?i)\\bgrue\\b"       # any line mentioning a "grue" → red (flavour example)
# fg = "red"

# ── Status bar. Omit [statusbar] for the built-in default. ───────────────────
# Placeholders: {location} {score} {moves} {time} {turns} {title} {filter}
# NB: this is lanthorn's OWN bar, and `border` frames that. The frame round a
# GAME's status/upper window is the upper_window_border line up in [elements].
# [statusbar]
# border = "none"

# [[statusbar.segment]]
# text  = "{location}"
# align = "left"
# parent = "accent"              # segments may reference a role or set fg/bg directly
# bold  = true

# [[statusbar.segment]]
# text  = "Score: {score}  Moves: {moves}"
# align = "right"

# [[statusbar.segment]]
# text  = "{time}"
# align = "right"
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::GhosttyScheme;
    use crate::theme::resolve::resolve_theme;
    use crate::theme::toml_schema::{self, ParsedStyle};
    use ratatui::style::Color;

    /// Every non-`Statusbar` registry row's local key must appear somewhere in
    /// the generated template (regression: a new row without a template line).
    #[test]
    fn template_covers_every_registry_selector() {
        let template = commented_template();
        for row in REGISTRY.iter().filter(|r| r.section != Section::Statusbar) {
            let leaf = row.name.rsplit('.').next().unwrap();
            assert!(
                template.contains(leaf),
                "registry row {:?} (leaf {leaf:?}) missing from commented_template()",
                row.name
            );
        }
    }

    /// The whole template is comments/blanks only, so it parses as an empty
    /// (all-default) document.
    #[test]
    fn template_parses_clean() {
        let parsed = toml_schema::parse(&commented_template());
        assert!(parsed.is_ok(), "template failed to parse: {parsed:?}");
    }

    /// Uncomment every real TOML line (section headers + `key = value` /
    /// `key = {..}` assignments) while leaving prose/blurb comments alone,
    /// then parse it.
    fn uncomment_toml_lines(template: &str) -> String {
        template
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix("# ") {
                    if rest.starts_with('[') || rest.contains(" = ") {
                        return rest.to_string();
                    }
                }
                line.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A `GhosttyScheme` matching `Roles::terminal_default()`/the resolver's
    /// default test fixture, so `resolve_theme` reproduces registry defaults.
    fn terminal_default_scheme() -> GhosttyScheme {
        let mut scheme = GhosttyScheme { foreground: Color::White, ..GhosttyScheme::default() };
        scheme.palette[3] = Color::Yellow;
        scheme.palette[6] = Color::Cyan;
        scheme.palette[8] = Color::DarkGray;
        scheme
    }

    /// Uncommenting the whole template and resolving it must reproduce the
    /// registry-default theme (spot-checked across sections).
    #[test]
    fn uncommented_template_resolves_to_registry_defaults() {
        let scheme = terminal_default_scheme();
        let uncommented = uncomment_toml_lines(&commented_template());
        let parsed = toml_schema::parse(&uncommented)
            .unwrap_or_else(|e| panic!("uncommented template failed to parse: {e:?}"));

        let seeded = resolve_theme(&scheme, &parsed);
        let default = resolve_theme(&scheme, &ParsedStyle::default());

        // roles: every one's template default must resolve back to the registry
        // default, so the hand-written [roles] block can't drift from the resolver.
        // SQ-0642: this list said "border" — not a role; the real role is "line",
        // whose template row was therefore never checked (both sides fell back to
        // the text fallback and compared equal vacuously). Pin it to ROLE_NAMES
        // so the list itself can't drift again.
        for role in super::super::registry::ROLE_NAMES {
            assert_eq!(seeded.get(role).style, default.get(role).style, "role {role} drifted");
        }
        // an element
        assert_eq!(seeded.get("transcript_meta").style, default.get("transcript_meta").style);
        assert_eq!(seeded.get("transcript_meta").glyph, default.get("transcript_meta").glyph);
        // panel.border style
        assert_eq!(seeded.get("panel.border").border, default.get("panel.border").border);
        assert_eq!(seeded.get("panel.border").style, default.get("panel.border").style);
        // a glk slot
        assert_eq!(seeded.get("glk.buffer.header").style, default.get("glk.buffer.header").style);
        // a map colour
        assert_eq!(seeded.get("map.connector_distorted").style, default.get("map.connector_distorted").style);
        // a debug tier
        assert_eq!(seeded.get("debug.disasm_data").style, default.get("debug.disasm_data").style);
        assert_eq!(seeded.get("debug.disasm_data").glyph, default.get("debug.disasm_data").glyph);
    }

    /// SQ-1105: a row whose `glyph` the renderer does not read says so.
    ///
    /// `transcript_meta`, `transcript_warning` and the two assist rows mirror the
    /// gutter default in their `glyph` deliberately — the registry keeps the two
    /// spellings within a sentence of each other. But the template is read by
    /// someone holding the key, not the registry, and an inert `glyph = "▏"`
    /// beside a live `parent = "muted"` is an invitation to set it and watch
    /// nothing happen. That is precisely what `customization.md` used to promise
    /// in as many words. The note is the whole fix; this stops it going quiet.
    #[test]
    fn a_glyph_the_renderer_ignores_is_labelled_colour_only() {
        let t = commented_template();
        for (row, key) in [
            ("transcript_meta", "gutter.meta"),
            ("transcript_warning", "gutter.warning"),
            ("transcript_assist", "gutter.assist"),
            ("transcript_assist_caution", "gutter.assist"),
        ] {
            let line = t
                .lines()
                .find(|l| l.trim_start_matches("# ").starts_with(&format!("{row} = ")))
                .unwrap_or_else(|| panic!("{row} is not in the template"));
            assert!(
                line.contains("colour only") && line.contains(key),
                "{row} advertises a glyph the renderer never reads, with no note saying \
                 the mark comes from {key}: {line:?}"
            );
        }
    }

    /// The repo-root `style.example.toml` must be exactly `commented_template()`'s
    /// output, so the checked-in example can never drift from the generator.
    /// Regenerate it with the generator's output whenever this fails.
    #[test]
    fn style_example_matches_generated_template() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        assert_eq!(text, commented_template(), "style.example.toml is stale — regenerate it from commented_template()");
    }

    // ── SQ-1170: every key the template documents, proven to land ─────────────
    //
    // `style::tests::style_example_toml_presets_take_effect_when_uncommented` is
    // the right shape and was written for exactly this failure — a key documented
    // under a section the parser does not read (SQ-0558/SQ-0559) — but it names
    // EIGHT keys by hand out of the 181 rows the generator documents. A
    // hand-written list cannot cover a row nobody remembers to add to it, which
    // is the lesson the full test gate learned when it named five crates instead
    // of `--workspace`. It also cost a real defect: a user's correct
    // `[tooltip] background = { parent = "dialog.list_selected" }` parsed,
    // resolved, warned about nothing, and changed no pixel, with every test green
    // (SQ-1169/`a9898db9`).
    //
    // So the sweep below is driven off `commented_template()` itself. For every
    // documented line it rewrites that ONE line uncommented, with a value the
    // default is not, and requires the artifact that line feeds to change.
    // Routing is by the row's `Kind` and the shape of its own default `Delta`,
    // never by name, so a row added to `REGISTRY` is covered the day it lands.
    //
    // What this proves and what it does not: that the key is READ — parsed,
    // lowered, resolved, and visible in the artifact a renderer reads from. That
    // some renderer then reads that selector is a different claim, and the one
    // `no_registry_row_is_documented_but_read_by_nothing` (registry.rs) makes.

    /// The live section headers the generator emits, paired with the [`Section`]
    /// each holds. `[map.overrides]` is deliberately absent: it is a free-form
    /// glyph table with no registry row, covered by
    /// `every_map_override_slot_the_example_file_documents_is_a_real_slot`.
    fn section_headers() -> Vec<(&'static str, Section)> {
        std::iter::once(ROLES_SECTION)
            .chain(TEXT_SECTIONS.iter().copied())
            .chain(SURFACE_SECTIONS.iter().copied())
            .map(|(section, header, _)| (header, section))
            .collect()
    }

    /// One documented value line: the registry row it was generated from, the
    /// TOML key it is written under, and which template line it is.
    struct Documented {
        row: &'static RegRow,
        key: String,
        line: usize,
    }

    /// Every documented `# key = value` line in the generated template, paired
    /// with the registry row it came from.
    ///
    /// Both directions are checked: a documented key with no row would be
    /// SQ-0561's typo'd slot in a new place, and a row with no documented line is
    /// a knob the shipped file never mentions.
    fn documented_lines(template: &str) -> Vec<Documented> {
        let registry: &'static Vec<RegRow> = &REGISTRY;
        let headers = section_headers();
        let mut found: Vec<Documented> = Vec::new();
        let mut section: Option<Section> = None;

        for (idx, line) in template.lines().enumerate() {
            // The hand-written `[[transcript.rule]]` / `[statusbar]` examples at
            // the foot of the file are COMMENTED headers, and nothing below them
            // is a registry row.
            if line.starts_with("# [") {
                break;
            }
            if line.starts_with('[') {
                section = headers.iter().find(|(h, _)| *h == line).map(|(_, s)| *s);
                continue;
            }
            let (Some(sect), Some(rest)) = (section, line.strip_prefix("# ")) else { continue };
            let Some((raw_key, _)) = rest.split_once('=') else { continue };
            let key = raw_key.trim();
            let row = registry
                .iter()
                .find(|r| r.section == sect && toml_key(&strip_section_prefix(sect, r.name)) == key)
                .unwrap_or_else(|| {
                    panic!(
                        "the template documents {key:?} under {sect:?}, and no registry row \
                         spells that key — nothing can read it"
                    )
                });
            found.push(Documented { row, key: key.to_string(), line: idx });
        }

        for row in registry.iter().filter(|r| r.section != Section::Statusbar) {
            assert!(
                found.iter().any(|d| d.row.name == row.name),
                "registry row {:?} has no documented line in the generated template",
                row.name
            );
        }
        found
    }

    /// The colour artifact a style.toml resolves to: the flat theme every
    /// renderer reads its styles from.
    fn theme_of(text: &str) -> crate::theme::resolve::Theme {
        let parsed = toml_schema::parse(text)
            .unwrap_or_else(|e| panic!("probe document failed to parse: {e:?}"));
        resolve_theme(&terminal_default_scheme(), &parsed)
    }

    /// The glyph artifact: the resolved map symbol set plus the three
    /// story-picker badge glyphs, which live on the `SymbolConfig` rather than
    /// the set.
    ///
    /// Deliberately NOT the whole `SymbolConfig` — that carries the raw preset
    /// NAMES, so `box_style = "bogus"` would look like a change while resolving
    /// to the default glyphs.
    fn glyphs_of(text: &str) -> (crate::symbols::SymbolSet, [String; 3]) {
        let doc = crate::style::parse_style_toml(text).expect("probe document must parse");
        let cfg = crate::style::finalize_symbols(&doc.symbols);
        let badges = [cfg.badge_save.clone(), cfg.badge_hint.clone(), cfg.badge_hint_available.clone()];
        (crate::symbols::SymbolSet::resolve(&cfg), badges)
    }

    /// Rewrite exactly one template line, leaving every other line commented.
    fn with_line(template: &str, idx: usize, replacement: &str) -> String {
        let mut lines: Vec<&str> = template.lines().collect();
        lines[idx] = replacement;
        lines.join("\n")
    }

    /// Every preset name every glyph family knows, asked of the families
    /// themselves rather than spelled here — a preset added to one is a
    /// candidate the day it lands. A [`Kind::Placement`] row names a preset from
    /// exactly one of these, and an unknown name resolves to that row's default,
    /// so trying the union finds the row's own family without a name table.
    fn every_preset_name() -> Vec<&'static str> {
        use crate::symbols::{Arrows, BoxStyle, ControlGlyphs, PathGlyphs, PortalGlyphs, StoryBadges};
        let mut names: Vec<&'static str> = Vec::new();
        for family in [
            BoxStyle::preset_names(),
            Arrows::preset_names(),
            PathGlyphs::preset_names(),
            PortalGlyphs::preset_names(),
            ControlGlyphs::preset_names(),
            StoryBadges::preset_names(),
        ] {
            names.extend(family.iter().copied());
        }
        names
    }

    #[test]
    fn every_key_the_template_documents_takes_effect_when_uncommented() {
        use ratatui::style::Modifier;

        let template = commented_template();
        let documented = documented_lines(&template);
        let base_theme = theme_of(&template);
        let base_glyphs = glyphs_of(&template);
        let presets = every_preset_name();

        for d in &documented {
            let name = d.row.name;
            let key = &d.key;
            let probe = |value: &str| with_line(&template, d.line, &format!("{key} = {value}"));
            let base = base_theme.get(name);

            // ── glyph-set presets: the map's box/arrow/path/portal/control sets
            // and the story-picker badges. These are read by the SYMBOL path, not
            // the theme, so the artifact is the resolved set, not a style.
            if d.row.kind == Kind::Placement {
                // The one row whose value a `Delta` cannot carry is the bool
                // (`map.diagonal_corners`) — told apart by the shape of its own
                // default rather than by its name.
                let values: Vec<String> = if d.row.default_delta.glyph.is_none() {
                    vec!["true".to_string(), "false".to_string()]
                } else {
                    presets
                        .iter()
                        .map(|n| format!("\"{n}\""))
                        // …and a free-form mark, for the three badge keys, which
                        // name no preset: any string a patched font can draw.
                        .chain(std::iter::once("\"¤\"".to_string()))
                        .collect()
                };
                assert!(
                    values.iter().any(|v| glyphs_of(&probe(v)) != base_glyphs),
                    "the template documents {key:?} for row {name:?}, but no value changes the \
                     resolved symbol set — nothing reads that key"
                );
                continue;
            }

            // ── colour channels: every selector accepts fg and bg, roles
            // included. Two candidates so a row whose default happens to be one
            // of them still has the other to move to.
            for chan in ["fg", "bg"] {
                let landed = ["#ff00ff", "#00ff7f"].iter().any(|c| {
                    let got = theme_of(&probe(&format!("{{ {chan} = \"{c}\" }}"))).get(name);
                    match chan {
                        "fg" => got.style.fg != base.style.fg,
                        _ => got.style.bg != base.style.bg,
                    }
                });
                assert!(
                    landed,
                    "the template documents {key:?} for row {name:?}, but setting {chan} on it \
                     changes no resolved colour"
                );
            }

            // Roles stop here. `lower_role_decls` reads fg/bg and drops the rest
            // — role modifiers and a role `parent` are a documented deferral
            // (resolve.rs: "Still deferred: role modifiers"), and a role has no
            // border or glyph to carry. Everything a `[roles]` line documents IS
            // an fg or a bg, so nothing here is exempted, only absent.
            if d.row.section == Section::Roles {
                continue;
            }

            // ── modifiers: set one the row does NOT already carry. The template
            // documents a modifier only where it is the DEFAULT (`bold = true` on
            // a row that is bold), and `apply_style` cannot CLEAR one, so the
            // documented spelling has no non-default value to write; probing a
            // free flag proves the same keys are read for this row.
            let carried = base.style.add_modifier;
            let free = [
                ("bold", Modifier::BOLD),
                ("italic", Modifier::ITALIC),
                ("underline", Modifier::UNDERLINED),
                ("reversed", Modifier::REVERSED),
                ("dim", Modifier::DIM),
            ]
            .into_iter()
            .find(|(_, m)| !carried.contains(*m));
            let (modifier, _) = free.unwrap_or_else(|| {
                panic!("row {name:?} carries every modifier by default; none is free to probe")
            });
            assert_ne!(
                theme_of(&probe(&format!("{{ {modifier} = true }}"))).get(name).style.add_modifier,
                carried,
                "the template documents {key:?} for row {name:?}, but setting {modifier} on it \
                 changes no resolved modifier"
            );

            // ── the border `style` key, which every surface selector accepts
            // (§2a) and which the generic line emits wherever a row's default
            // carries one.
            let landed = ["double", "thick", "single", "none", "rounded"]
                .iter()
                .any(|s| theme_of(&probe(&format!("{{ style = \"{s}\" }}"))).get(name).border != base.border);
            assert!(
                landed,
                "the template documents {key:?} for row {name:?}, but no border style set on it \
                 changes the resolved border"
            );

            // ── the `glyph` key (gutter marks, tab dividers, terminator caps).
            let landed = ["\u{2588}", "\u{00A4}"]
                .iter()
                .any(|g| theme_of(&probe(&format!("{{ glyph = \"{g}\" }}"))).get(name).glyph != base.glyph);
            assert!(
                landed,
                "the template documents {key:?} for row {name:?}, but setting a glyph on it \
                 changes no resolved glyph"
            );

            // ── `parent`: the re-root channel a user reaches for to move a
            // whole family at once, and the one the tooltip defect hid in.
            //
            // The bar is that re-rooting must move at least one COLOUR — not
            // that it must move every colour, and not merely that it changes the
            // style, which a bold `heading` parent would satisfy while the
            // colours stayed pinned (that weaker bar passed the tooltip defect
            // itself, so it is no bar at all). SQ-1169 made this bar clearable
            // for every row: `resolve_row` now skips the registry default
            // delta's `fg`/`bg` on any row a user layer re-roots, so a pinned
            // colour no longer survives on top of the new parent.
            let landed = super::super::registry::ROLE_NAMES.iter().any(|p| {
                let got = theme_of(&probe(&format!("{{ parent = \"{p}\" }}"))).get(name);
                got.style.fg != base.style.fg || got.style.bg != base.style.bg
            });
            assert!(
                landed,
                "the template documents `parent` on row {name:?}, but re-rooting it onto any of \
                 the seven roles changes nothing — see SQ-1169's re-root rule in resolve_row"
            );
        }
    }

    #[test]
    fn auto_seed_writes_when_missing_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "lanthorn-template-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = personal_style_path(&dir);

        assert!(!path.exists());
        auto_seed(&dir);
        assert!(path.exists());
        let seeded = std::fs::read_to_string(&path).unwrap();
        assert_eq!(seeded, commented_template());

        // Modify the file, then seed again — must be left byte-unchanged.
        std::fs::write(&path, "# user was here\n").unwrap();
        auto_seed(&dir);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "# user was here\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
