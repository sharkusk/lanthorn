//! The selector registry: one declarative row per themeable selector.
//!
//! This is a *pure data table* — no behaviour is wired to it here. The resolver,
//! TOML parser, template generator and export path (later SQ-0309 tasks) all read
//! [`REGISTRY`] so that every themeable element is declared exactly once.
//!
//! Each [`RegRow`] carries the selector's `name`, its template [`Section`], its
//! [`Kind`], its `parent` (a role name or another selector), and the
//! `default_delta` layered on that parent. Values are transcribed from the design
//! spec `docs/design/2026-07-14-styling-role-redesign.md` (§1–§4b) — that document
//! is authoritative for every parent/delta.

use ratatui::style::Color;

/// Which template section a selector belongs to (drives grouping in the seed
/// template / `style.example.toml`). Every row has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The 7 role roots (§1).
    Roles,
    /// App chrome/text elements (§2).
    Elements,
    /// Shared panel chrome (§2a/§2b).
    Panel,
    /// The 11 Glk styles for text-buffer windows (§3).
    GlkBuffer,
    /// The 11 Glk styles for text-grid windows (§3).
    GlkGrid,
    /// Map-domain colours + glyph-set presets (§4).
    Map,
    /// Debug inspector disassembly selectors (§4b).
    Debug,
    /// Modal dialog surface: background, border, title, buttons, shadow.
    Dialog,
    /// Hover-tooltip surface: background, border.
    Tooltip,
    /// Status-bar segments (dynamic `[[statusbar.segment]]`; no fixed rows here).
    Statusbar,
}

/// What a selector primarily controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain text/colour style (fg/bg + modifiers).
    Style,
    /// A border/frame or a glyph carried in a border (dividers, terminators).
    BorderGlyphs,
    /// A named glyph-set preset (e.g. map `box_style = "rounded"`).
    Placement,
}

/// A partial style layered on a selector's parent: optional fg/bg, the terminal
/// modifier flags, and optional glyph(s).
///
/// - `glyph` is a single glyph a selector carries (gutter marks, tab dividers,
///   terminator caps) or, for a [`Kind::Placement`] preset row, the preset name.
///
/// There is deliberately no named-slot list: a `glyphs = { tl = "+" }` sub-map
/// used to parse into every layer and then reach no renderer (SQ-0560). Single
/// map glyphs are overridden through `[map.overrides]`.
///
/// Owns its string/glyph channels (not `&'static`) so a runtime user override —
/// parsed from `style.toml` and lowered into a layer `Delta` — uses the same type
/// as the registry defaults (SQ-0440). `border` carries a resolved
/// [`BorderStyle`]; `parent` re-roots this selector's inheritance when a user sets
/// it (the registry default parent lives on [`RegRow`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    // ── Modifiers are TRI-STATE, and that is the whole point of the Option ────
    //
    // `None` inherits the parent's, `Some(true)` adds, `Some(false)` REMOVES.
    // They were plain `bool` until SQ-1171, which composed additively and so
    // gave a child no way to shed a modifier its parent set: `bold = false` in
    // `style.toml` was a silent no-op rather than an eraser, and a row could
    // only ever accumulate weight as it inherited.
    //
    // `RawDelta` has always parsed these as `Option<bool>` — the TOML layer
    // could tell "absent" from "false" the whole time — and the distinction was
    // thrown away by an `unwrap_or(false)` when lowering into this type. So this
    // is less a new feature than the parser's own answer stopping being
    // discarded on the way in.
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub reversed: Option<bool>,
    pub dim: Option<bool>,
    pub glyph: Option<String>,
    pub border: Option<crate::render::paneframe::BorderStyle>,
    /// Per-side border-style overrides (SQ-0641): a set side wins over `border`
    /// for that edge. Lowered from a decl's `style_top`/`style_bottom`/
    /// `style_left`/`style_right` keys, which used to parse and then be dropped.
    pub border_top: Option<crate::render::paneframe::BorderStyle>,
    pub border_bottom: Option<crate::render::paneframe::BorderStyle>,
    pub border_left: Option<crate::render::paneframe::BorderStyle>,
    pub border_right: Option<crate::render::paneframe::BorderStyle>,
    /// Whether the pane's header strip is shown (`header = true/false`).
    pub header: Option<bool>,
    /// Whether the surface draws a drop shadow (`shadow = true/false`).
    pub shadow: Option<bool>,
    pub parent: Option<String>,
}

impl Delta {
    /// The zero delta: same as the parent, no glyph.
    pub const EMPTY: Delta = Delta {
        fg: None,
        bg: None,
        bold: None,
        italic: None,
        underline: None,
        reversed: None,
        dim: None,
        glyph: None,
        border: None,
        border_top: None,
        border_bottom: None,
        border_left: None,
        border_right: None,
        header: None,
        shadow: None,
        parent: None,
    };
}

/// One registry row: a themeable selector and how it derives from its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct RegRow {
    pub name: &'static str,
    pub section: Section,
    pub kind: Kind,
    pub parent: Option<&'static str>,
    pub default_delta: Delta,
}

/// The 7 role root names (§1). Roles are the parents everything else derives
/// from; their concrete colours come from the base scheme + `[roles]`, not this
/// table, so their rows carry `parent = None` and an empty delta.
pub const ROLE_NAMES: [&str; 7] =
    ["text", "chrome", "line", "accent", "muted", "alert", "heading"];

/// The default for `map.diagonal_corners` (§4) — the sole bool-valued selector,
/// so its value lives here rather than in a [`Delta`]. Must equal
/// `config::default_diagonal_corners()`.
///
/// SQ-0641: `map.layer_cycle` (a list-valued per-layer edge-colour cycle) used
/// to sit beside this. It was documented in the shipped template but had no
/// consumer anywhere — no renderer cycles edge colours per layer (layer tabs are
/// styled via `panel.tab`/`panel.tab:active`) — so the row, its default and its
/// template line were removed rather than shipping a knob that does nothing.
pub const DIAGONAL_CORNERS_DEFAULT: bool = true;

/// Build a row concisely.
fn row(
    name: &'static str,
    section: Section,
    kind: Kind,
    parent: Option<&'static str>,
    default_delta: Delta,
) -> RegRow {
    RegRow { name, section, kind, parent, default_delta }
}

/// A delta carrying only modifier flags (colours/glyph inherited from parent).
///
/// `false` here means "not mentioned", NOT "remove it" — every existing call site
/// passes `false` for the modifiers it simply does not care about, and reading
/// those as erasures would have each row strip its parent's weight. A row that
/// genuinely wants to shed one spells `Some(false)` in a `Delta` literal, which
/// is rare enough to be worth seeing at the site.
fn mods(bold: bool, italic: bool, underline: bool, reversed: bool) -> Delta {
    let set = |b: bool| if b { Some(true) } else { None };
    Delta {
        bold: set(bold),
        italic: set(italic),
        underline: set(underline),
        reversed: set(reversed),
        ..Delta::EMPTY
    }
}

/// A delta setting only a foreground colour.
fn fg(c: Color) -> Delta {
    Delta { fg: Some(c), ..Delta::EMPTY }
}

/// A delta carrying only a single glyph (e.g. gutter mark, tab divider, or a
/// [`Kind::Placement`] preset name).
fn glyph(g: &str) -> Delta {
    Delta { glyph: Some(g.to_string()), ..Delta::EMPTY }
}

/// A delta setting only the border style (colour/glyph inherited from parent).
fn border(name: &str) -> Delta {
    Delta { border: Some(crate::render::paneframe::parse_border_style(name)), ..Delta::EMPTY }
}

/// The registry table. Built once at first use: `Delta` now owns its glyph/border
/// channels (SQ-0440), so the rows can no longer be a compile-time `const`.
pub static REGISTRY: std::sync::LazyLock<Vec<RegRow>> = std::sync::LazyLock::new(|| vec![
    // ── §1 roles ──────────────────────────────────────────────────────────────
    // Roots: colours come from the base scheme + `[roles]`, so no parent/delta.
    row("text", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("chrome", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("line", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("accent", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("muted", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("alert", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("heading", Section::Roles, Kind::Style, None, Delta::EMPTY),
    // ── §2 elements ───────────────────────────────────────────────────────────
    row("transcript", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("status_bar", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("help_bar", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("upper_window", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("story_info", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    // heading on chrome: heading fg with the chrome bg (black).
    row("status_header", Section::Elements, Kind::Style, Some("heading"), Delta { bg: Some(Color::Black), ..Delta::EMPTY }),
    row("story_title", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    row("input_line", Section::Elements, Kind::Style, Some("line"), Delta::EMPTY),
    row("suggestion_line", Section::Elements, Kind::Style, Some("line"), Delta::EMPTY),
    // The scrollbar is two BACKGROUND fills, no glyphs (SQ-0782): `scrollbar`
    // colours the thumb, `scrollbar_track` the dim channel it runs in. Each
    // selector's FG is its fill, so an existing `scrollbar = { fg = … }` still
    // means "the colour of the bar".
    row("scrollbar", Section::Elements, Kind::Style, Some("line"), Delta::EMPTY),
    row("scrollbar_track", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("transcript_location", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("story_badge", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    // The badge GLYPHS sit beside the `story_badge` selector that colours them —
    // one concept, one place (SQ-0559). Presets, so the letter rides in `glyph`.
    // `badge_icons` chooses the SET (SQ-1159) and the font check writes it; the
    // keys under it override one badge each. SQ-1160 retired the story-type and
    // Blorb badges: SQ-0369 had moved both into the row's TYPE column as text.
    row("badge_icons", Section::Elements, Kind::Placement, None, glyph("plain")),
    row("badge_save", Section::Elements, Kind::Placement, None, glyph("S")),
    row("badge_hint", Section::Elements, Kind::Placement, None, glyph("H")),
    row("badge_hint_available", Section::Elements, Kind::Placement, None, glyph("h")),
    row("hyperlink", Section::Elements, Kind::Style, Some("accent"), mods(false, false, true, false)),
    row("story_info_label", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("suggestion", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // transcript_meta / transcript_warning carry a gutter glyph (§2/§4 glyph decision).
    row("transcript_meta", Section::Elements, Kind::Style, Some("muted"), glyph("▏")),
    row("transcript_warning", Section::Elements, Kind::Style, Some("alert"), glyph("!")),
    row("transcript_crash", Section::Elements, Kind::Style, Some("alert"), mods(true, false, false, false)),
    // SQ-1045: Lanthorn's Guiding Light. A light is YELLOW, and the yellow slot is
    // `alert` — so the two assist tones share that role and separate by WEIGHT,
    // exactly as `transcript_crash` (bold `alert`) separates from
    // `transcript_warning` (plain `alert`) two rows up, and as `glk.buffer.alert`
    // does from its own. An eighth role would be a new name for a colour the
    // terminal already has one of.
    //
    // A role rather than an RGB literal is the whole reason the change is safe on
    // both grounds: yellow resolves through the player's own palette, so it is
    // Solarized Light's `#b58900` on `#fdf6e3` and their scheme's yellow on a dark
    // page, with no lightness test in lanthorn (see `colors.rs`'s note that the
    // accents are chosen to be legible on any page).
    //
    // The glyph is the MARK — the icon that identifies an assist on screen now
    // that the text carries no prefix. `symbols.assist_gutter` is what the
    // renderer actually draws; this keeps the two spellings of the default in one
    // sentence of each other.
    row("transcript_assist", Section::Elements, Kind::Style, Some("alert"), glyph("●")),
    row(
        "transcript_assist_caution",
        Section::Elements,
        Kind::Style,
        Some("alert"),
        Delta { glyph: Some("●".to_string()), ..mods(true, false, false, false) },
    ),
    // SQ-1107: the momentary reveal's lit words. `accent` and UNDERLINED, not the
    // assist's yellow — this is ink laid over the STORY's own prose for a few
    // seconds, so it has to read against whatever colour the game already painted
    // those words, and an underline survives a ground a foreground cannot. The
    // `alert` slot belongs to the Guiding Light's own lines and its gutter mark,
    // and a reveal is not one of those lines.
    row("transcript_reveal", Section::Elements, Kind::Style, Some("accent"), mods(false, false, true, false)),
    // ── §2a/§2b panel.* (shared panel chrome) ────────────────────────────────
    // Transparent by default (no parent/bg) so panels show the terminal
    // background out of the box; set a bg here to give panels a solid surface.
    row("panel.background", Section::Panel, Kind::Style, None, Delta::EMPTY),
    // border rows: default border style = "single" (unified panel frame, §2a).
    // `:active` = single + bold (today's cyan+bold).
    row("panel.border", Section::Panel, Kind::BorderGlyphs, Some("line"), border("single")),
    row("panel.border:active", Section::Panel, Kind::BorderGlyphs, Some("line"),
        Delta { border: Some(crate::render::paneframe::BorderStyle::Single), bold: Some(true), ..Delta::EMPTY }),
    row("panel.title", Section::Panel, Kind::Style, Some("heading"), Delta::EMPTY),
    row("panel.tab", Section::Panel, Kind::Style, Some("muted"), Delta::EMPTY),
    row("panel.tab:active", Section::Panel, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("panel.tab_divider", Section::Panel, Kind::BorderGlyphs, Some("line"), glyph("│")),
    // terminator glyphs default to the single-border caps ┤/├ (spec §2b).
    row("panel.terminator_left", Section::Panel, Kind::BorderGlyphs, Some("line"), glyph("┤")),
    row("panel.terminator_right", Section::Panel, Kind::BorderGlyphs, Some("line"), glyph("├")),
    // SQ-1123: the clickable toggle controls on a pane's frame. Three states,
    // because a control that looks the same on and off is half a control:
    // `control` is the OFF/idle one, `:lit` is EVERY on state — the `alert`
    // role, the same yellow slot `transcript_assist` lights up in, so the
    // Guiding Light and its own mark in the gutter are one colour — and
    // `:hover` is the pointer's, which wins over both so a control the pointer
    // is on always reads as reachable whether or not it is on.
    //
    // There was a `:active` beside `:lit` while "on" and "lit" were different
    // states. Every on state is lit now, so nothing could resolve to it, and a
    // selector a themer can set and never see is worse than one that is absent.
    row("panel.control", Section::Panel, Kind::Style, Some("muted"), Delta::EMPTY),
    row("panel.control:lit", Section::Panel, Kind::Style, Some("alert"), mods(true, false, false, false)),
    row("panel.control:hover", Section::Panel, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // ── §3 glk.buffer.* (buffer base = text) ─────────────────────────────────
    row("glk.buffer.normal", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.emphasized", Section::GlkBuffer, Kind::Style, Some("text"), mods(false, true, false, false)),
    row("glk.buffer.preformatted", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.header", Section::GlkBuffer, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.buffer.subheader", Section::GlkBuffer, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.buffer.alert", Section::GlkBuffer, Kind::Style, Some("alert"), mods(true, false, false, false)),
    row("glk.buffer.note", Section::GlkBuffer, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("glk.buffer.blockquote", Section::GlkBuffer, Kind::Style, Some("muted"), Delta::EMPTY),
    row("glk.buffer.input", Section::GlkBuffer, Kind::Style, Some("accent"), Delta::EMPTY),
    row("glk.buffer.user1", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.user2", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    // ── §3 glk.grid.* (grid base = chrome) ───────────────────────────────────
    row("glk.grid.normal", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.emphasized", Section::GlkGrid, Kind::Style, Some("chrome"), mods(false, true, false, false)),
    row("glk.grid.preformatted", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.header", Section::GlkGrid, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.grid.subheader", Section::GlkGrid, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.grid.alert", Section::GlkGrid, Kind::Style, Some("alert"), mods(true, false, false, false)),
    row("glk.grid.note", Section::GlkGrid, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("glk.grid.blockquote", Section::GlkGrid, Kind::Style, Some("muted"), Delta::EMPTY),
    row("glk.grid.input", Section::GlkGrid, Kind::Style, Some("accent"), Delta::EMPTY),
    row("glk.grid.user1", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.user2", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    // ── §4 map.* colours (hybrid (c): own tokens; current/selected → accent) ──
    // map.background: own token, defaults to terminal background (resolver-supplied).
    row("map.background", Section::Map, Kind::Style, None, Delta::EMPTY),
    // Room fill + cardinal/portal connectors derive from roles so a base scheme
    // recolours them (old from_ghostty: room=foreground, connector=palette[6]).
    // `text`/`accent` give white/cyan for the terminal default AND follow the scheme.
    row("map.room", Section::Map, Kind::Style, Some("text"), Delta::EMPTY),
    row("map.room_current", Section::Map, Kind::Style, Some("accent"), Delta::EMPTY),
    row("map.room_selected", Section::Map, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("map.connector", Section::Map, Kind::Style, Some("accent"), Delta::EMPTY),
    // `distorted` (magenta) has no matching role — kept explicit (a distinctive marker).
    row("map.connector_distorted", Section::Map, Kind::Style, None, fg(Color::Magenta)),
    row("map.connector_portal", Section::Map, Kind::Style, Some("accent"), Delta::EMPTY),
    row("map.shared_path", Section::Map, Kind::Style, None, fg(Color::LightCyan)),
    // The `?` marking a compass direction never tried from a room (SQ-0391). `muted` on purpose:
    row("map.loc_indicator", Section::Map, Kind::Style, Some("muted"), Delta::EMPTY),
    // ── The matrix view (SQ-0666). Defaults reproduce the design's mockup: a heading row,
    // the standing-in room reversed like a selected room, the selected row accented, its
    // entrances elsewhere BOLD (the cross-highlight is style, never a glyph, so the table's
    // vocabulary stays one glyph per fact), and the `·`/`×` frontier dimmed out of the way.
    row("map.matrix.header", Section::Map, Kind::Style, Some("heading"), Delta::EMPTY),
    row("map.matrix.row:here", Section::Map, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("map.matrix.row:selected", Section::Map, Kind::Style, Some("accent"), Delta::EMPTY),
    row("map.matrix.cell:entrance", Section::Map, Kind::Style, Some("text"), mods(true, false, false, false)),
    // The route from where you are standing to the room you clicked (SQ-0693): one cell per step,
    // the direction you LEAVE BY. Underlined rather than bold, because it must not be confused
    // with the entrance bolding beside it — that answers "how do I get BACK here", this answers
    // "how do I get THERE", and the two frequently light up in the same row.
    row("map.matrix.cell:path", Section::Map, Kind::Style, Some("accent"), mods(true, false, true, false)),
    row("map.matrix.cell:frontier", Section::Map, Kind::Style, Some("muted"), Delta::EMPTY),
    row("map.matrix.footnote", Section::Map, Kind::Style, Some("muted"), Delta::EMPTY),
    // ── Honest asymmetric edges in the DRAWN view (SQ-0666). Both default to the current
    // connector appearance, so nothing changes look until someone styles them.
    row("map.edge:oneway", Section::Map, Kind::Style, Some("map.connector"), Delta::EMPTY),
    row("map.edge:asym", Section::Map, Kind::Style, Some("map.connector"), Delta::EMPTY),
    // The last few passages walked, on a maze layer — a breadcrumb, so: quiet.
    row("map.trail", Section::Map, Kind::Style, Some("muted"), Delta::EMPTY),
    // Glyph-set presets (the old [symbols] section, merged in): preset name in `glyph`.
    row("map.box_style", Section::Map, Kind::Placement, None, glyph("rounded")),
    row("map.arrow_set", Section::Map, Kind::Placement, None, glyph("filled")),
    row("map.portal_icons", Section::Map, Kind::Placement, None, glyph("ascii")),
    row("map.path_style", Section::Map, Kind::Placement, None, glyph("light")),
    row("map.portal_path_style", Section::Map, Kind::Placement, None, glyph("dotted")),
    row("map.control_icons", Section::Map, Kind::Placement, None, glyph("plain")),
    // diagonal_corners is the one map knob that is a BOOL, not a preset name.
    // `Delta`'s value channels (colours, text modifiers, `glyph`) cannot carry
    // it, so the row exists for completeness and its value lives in
    // `DIAGONAL_CORNERS_DEFAULT`, emitted by a `template::row_line` case.
    row("map.diagonal_corners", Section::Map, Kind::Placement, None, Delta::EMPTY),
    // ── §4b debug.* (disasm-only; each tier carries a gutter glyph) ───────────
    row("debug.pc", Section::Debug, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // Default tier colours read as a risk gradient: blue = verified (executed),
    // yellow = medium (RD-discovered), red = high risk (unverified linear scan).
    row("debug.disasm_executed", Section::Debug, Kind::Style, Some("accent"), Delta { fg: Some(Color::Blue), glyph: Some("|".to_string()), ..Delta::EMPTY }),
    row("debug.disasm_rd", Section::Debug, Kind::Style, Some("text"), Delta { fg: Some(Color::Yellow), glyph: Some(" ".to_string()), ..Delta::EMPTY }),
    row("debug.disasm_soft", Section::Debug, Kind::Style, Some("muted"), Delta { fg: Some(Color::Red), glyph: Some(" ".to_string()), ..Delta::EMPTY }),
    row("debug.disasm_data", Section::Debug, Kind::Style, Some("muted"), Delta { italic: Some(true), glyph: Some(" ".to_string()), ..Delta::EMPTY }),
    // The Memory view's decoded-Z-text column (SQ-0448/SQ-0969): story text, not
    // a disassembly tier, so it takes `accent` and italics to read as a gloss on
    // the dump rather than as more of it.
    row("debug.zstring", Section::Debug, Kind::Style, Some("accent"), Delta { italic: Some(true), ..Delta::EMPTY }),
    // ── §2c dialog.* (modal surface: background + own frame + title/buttons/shadow) ──
    row("dialog.background", Section::Dialog, Kind::Style, Some("chrome"), Delta::EMPTY),
    // Modal frame: its own border (was panel.border:active); modals are always
    // "active", so single + bold reproduces today's cyan+bold dialog frame.
    row("dialog.border", Section::Dialog, Kind::BorderGlyphs, Some("line"), Delta { border: Some(crate::render::paneframe::BorderStyle::Single), bold: Some(true), ..Delta::EMPTY }),
    row("dialog.title", Section::Dialog, Kind::Style, Some("accent"), Delta::EMPTY),
    row("dialog.button", Section::Dialog, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("dialog.button:active", Section::Dialog, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // dialog.shadow: the one explicit-colour row — keeps a distinctive dark-gray bg.
    row("dialog.shadow", Section::Dialog, Kind::Style, Some("muted"), Delta { bg: Some(Color::DarkGray), ..Delta::EMPTY }),
    // ── §2d tooltip.* (hover-tooltip surface: background + optional frame) ─────
    // A tooltip has to be a SURFACE, and `chrome` is not one: it resolves to the
    // page's own fg/bg, so a borderless tip was painted in exactly the colours it
    // floated over and was invisible except for its text (SQ-1139). It reported as
    // "blends into the background" because it *was* the background.
    //
    // `accent` is not one either, and that is the trap worth naming: the cyan a
    // tooltip wants IS accent's cyan, but accent is `fg(Cyan)` with NO background
    // (`slot(6, ..)` in resolve.rs), so deriving from it reproduces SQ-1139 exactly
    // — cyan ink on whatever the tip floats over. A surface needs the PAIR.
    //
    // `dialog.list_selected` is that pair: it is the Black-on-Cyan row highlight
    // every modal list already uses (Saves, Replay, browser, Config, verb dock), so
    // a tip now reads as the same "this one" surface the menus use, and retuning the
    // highlight moves the tooltip with it instead of leaving a warm-dark card behind.
    //
    // IT TAKES THE COLOURS AND NOT THE WEIGHT. The highlight is bold, but that bold
    // is not a decision anyone made: it predates the registry, inherited verbatim
    // from six hand-written `Style::new().fg(Black).bg(Cyan).add_modifier(BOLD)`
    // literals that SQ-0643 consolidated into one row, preserving appearance rather
    // than re-deciding it. On a single selected ROW it reads as "this one" and is
    // harmless; on a multi-line tooltip CARD it is a bold paragraph, which is a
    // different thing. So this row spells `bold: Some(false)` — the tri-state's
    // erase arm (SQ-1171), which did not exist when this parent was chosen.
    //
    // What IS load-bearing next door, and must not be swept up with the weight: the
    // highlight is a fg/bg SWAP and not `REVERSED`, because the command band's hover
    // style is `REVERSED` and both can be on screen at once
    // (`hover_style_is_reversed_and_distinct_from_the_column_selected_style`).
    //
    // WHY NO EXPLICIT fg/bg: a row whose default Delta pins colours cannot be
    // re-rooted from `style.toml` at all. `resolve_row` applies this Delta on top of
    // the parent BEFORE the user's decls, and a decl that sets only `parent` never
    // touches the colour channels — so the pin wins and the re-root is a silent
    // no-op. That is what the old literal pair did to
    // `background = { parent = "dialog.list_selected" }`. Inheriting instead of
    // pinning is what makes the line in `style.toml` actually work.
    row(
        "tooltip.background",
        Section::Tooltip,
        Kind::Style,
        Some("dialog.list_selected"),
        Delta { bold: Some(false), ..Delta::EMPTY },
    ),
    // Borderless by default (style = "none"); set a style to frame the tooltip.
    row("tooltip.border", Section::Tooltip, Kind::BorderGlyphs, Some("line"), border("none")),
    // ── §2 elements (expansion, Task 5.0): every remaining ColorScheme field ──
    row("more_prompt", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("tidy_progress", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("meta_marker", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("inventory_dock", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    // ── The room dock (SQ-0692): the docked panel describing one room. `room_dock`
    // is its body text; the header line naming the room, its layer and the
    // follow/pin regime gets its own selector, reversed while PINNED so the one
    // line that changes meaning is the one that changes look.
    row("room_dock", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("room_dock.header", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    row("room_dock.header:pinned", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("story_info_title", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    // ── `/dump-terminal` (SQ-0994). Its whole point is telling a MEASURED value
    // from an ASSUMED one, so the two get different looks: a heading to find the
    // section by, and `terminal_dump_assumed` on every line carrying a number
    // lanthorn guessed or could not reach. Ordinary values stay on
    // `transcript_meta` with the rest of the dump family.
    row("terminal_dump_heading", Section::Elements, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("terminal_dump_assumed", Section::Elements, Kind::Style, Some("alert"), Delta::EMPTY),
    row("story_info_value", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_info_blurb", Section::Elements, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("story_info_link", Section::Elements, Kind::Style, Some("accent"), mods(false, false, true, false)),
    row("story_info_cover", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    // SQ-0861: the marker in the two-column indent of an info-panel row that is
    // the wrapped tail of the line above it. Muted, because it is punctuation
    // saying "still the same field" — the wrapped text itself keeps whatever
    // style its logical line carries.
    row("story_info_continuation", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // SQ-0789: the info panel's read-only list of picture archives detected for
    // the selected story — the same list the launch-options dialog offers. Plain
    // value text, because it is inventory; the `:active` variant marks the one
    // the game's own config actually names, which is the single row in the block
    // that is a statement of fact rather than an option.
    row("story_info_artwork", Section::Elements, Kind::Style, Some("story_info_value"), Delta::EMPTY),
    row("story_info_artwork:active", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("graphics", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("inline_image", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("story_header", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("story_header_active", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("story_author", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_year", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_rating", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_no_metadata", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("story_tile", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_tile_selected", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, true)),
    // A folder row in the story picker (a sub-directory of the library, or `..`).
    row("story_folder", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("notification", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("hotkey_key", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    // Sound-beep pulse colours are bespoke (warm amber / cool blue) — no role
    // reproduces them, so they carry explicit fg like map.connector_distorted.
    row("sound_beep_high", Section::Elements, Kind::Style, None, fg(Color::Rgb(255, 180, 40))),
    row("sound_beep_low", Section::Elements, Kind::Style, None, fg(Color::Rgb(60, 140, 220))),
    row("transcript_input", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("transcript_system", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("warning_marker", Section::Elements, Kind::Style, Some("alert"), Delta::EMPTY),
    row("input_text", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("input_prompt", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("upper_window_border", Section::Elements, Kind::Style, Some("line"), Delta::EMPTY),
    row("room_panel", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // ── command palette (SQ-0419) — reuses the dialog chrome; these style its
    // rows. `palette_match` highlights the fuzzy-matched characters; `palette_selected`
    // is the highlighted row; `palette_query` is the input line. ──────────────
    row("palette_query", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("palette_name", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("palette_match", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("palette_desc", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("palette_selected", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // ── IFDB search modal (SQ-0413) — reuses the dialog chrome; these style its
    // result/file rows, the rating tail, the download glyph, and the attribution. ──
    row("ifdb_result", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("ifdb_result_selected", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, true)),
    row("ifdb_result_meta", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("ifdb_download_marker", Section::Elements, Kind::Style, Some("accent"), glyph("⭳")),
    // A file the download directory already holds (SQ-0597): its own glyph and a
    // muted tint, so "I have this one" reads without displacing the filename.
    row("ifdb_download_present", Section::Elements, Kind::Style, Some("muted"), glyph("✓")),
    row("ifdb_attribution", Section::Elements, Kind::Style, Some("muted"), mods(false, true, false, false)),
    // ── saves manager (SQ-0531) — the Type cell's portability tint. A "Game"
    // save carries standard save-instruction-PC bytes any interpreter reads and
    // gets the accent + the ↗ glyph; a host "State" snapshot stays here. ──
    row("saves_portable", Section::Elements, Kind::Style, Some("accent"), glyph("↗")),
    row("saves_host_only", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // ── SQ-0643: hard-coded modal/list styles → themed selectors ─────────────
    // The Black-on-Cyan-Bold row highlight shared by every modal list (Saves,
    // Replay, file picker/browser, Config, and the verb dock): one family
    // instead of six one-off `Style::new()` literals.
    row("dialog.list_selected", Section::Dialog, Kind::Style, None,
        Delta { fg: Some(Color::Black), bg: Some(Color::Cyan), bold: Some(true), ..Delta::EMPTY }),
    // Dim hint/footer text on a dialog surface (key-hint footers, the empty-list
    // hint, a turn's "(no output)" line). `muted` already resolves to dark-gray.
    row("dialog.list_footer", Section::Dialog, Kind::Style, Some("muted"), Delta::EMPTY),
    // The Settings screen's underlined column-header row: plain text, underlined.
    row("dialog.list_header", Section::Dialog, Kind::Style, Some("text"), mods(false, false, true, false)),
    // The hints panel's dim "this game has its own hints" suggestion line.
    row("dialog.hint_suggestion", Section::Dialog, Kind::Style, Some("alert"), Delta { dim: Some(true), ..Delta::EMPTY }),
    // SQ-1170 retired `dialog.input_preview`. It styled the verb menu's
    // composed-input line ("Input: <text>_"); `d5524618` replaced the verb menu
    // with the command band, which deliberately has no phrase row at all — a
    // player composes on the real story input line, styled by `input_line`
    // (SQ-0667). The row outlived its only consumer by five months, documented
    // and parsed and resolved the whole time, and reaching no pixel. There is no
    // shim: an old style.toml that still names it is dropped by the same
    // unrecognised-key path every retired key takes.
    // SQ-0789: the launch-options dialog's caveat lines — what a chosen art
    // rendition will and won't do (EGA's dither not yet fusing at 1:1), and
    // when a choice needs the checkbox to take effect. `alert` without `dim`:
    // these are the lines that stop a user picking a rendition and being puzzled
    // by the result, so they must not read as decoration.
    row("dialog.launch_caveat", Section::Dialog, Kind::Style, Some("alert"), Delta::EMPTY),
    // ── SQ-0439: the region prompt — the modal that asks whether a set of rooms
    // wants a layer of its own, and that picks between passages or destinations
    // when a `move-region` cannot settle one for itself. It reuses the dialog
    // chrome; these style its contents. The chosen row borrows the shared modal
    // list highlight rather than minting a seventh look for "this is the one".
    row("dialog.region_prompt.body", Section::Dialog, Kind::Style, Some("dialog.background"), Delta::EMPTY),
    // SQ-0858 split the room block in two: `rooms` is the COUNT — the "4 rooms:" header and the
    // "…and N more" tail — and `room` is one named room in the bulleted list beneath it, so the
    // names can be lit or dimmed apart from the arithmetic about them. It inherits `rooms`, so a
    // theme that only knows the old selector still styles the whole block as one.
    row("dialog.region_prompt.rooms", Section::Dialog, Kind::Style, Some("dialog.list_footer"), Delta::EMPTY),
    row("dialog.region_prompt.room", Section::Dialog, Kind::Style, Some("dialog.region_prompt.rooms"), Delta::EMPTY),
    row("dialog.region_prompt.option", Section::Dialog, Kind::Style, Some("dialog.background"), Delta::EMPTY),
    row("dialog.region_prompt.option:chosen", Section::Dialog, Kind::Style, Some("dialog.list_selected"), Delta::EMPTY),
    // ── SQ-1104: the first-run font check's two sample rows. Their own selector
    // because they are the QUESTION, not prose about it — a theme that dims the
    // dialog body must still be able to leave the glyphs at full strength, since
    // the whole ask is "can you see these clearly".
    row("dialog.font_check.sample", Section::Dialog, Kind::Style, Some("dialog.background"), Delta::EMPTY),
    // ── SQ-0664: the command band (bottom dock). Its rows reuse
    // `dialog.list_selected`. SQ-0667 (2026-08-05) retired the band's own
    // frame (it draws no `panel.border` anymore — see `render/command_band.rs`)
    // and its phrase line (`band.phrase` / `band.phrase:armed`, RETIRED along
    // with it — composing happens on the real story input line now, which has
    // no dedicated "armed" affordance; see the design doc's amendment). ──────
    // Column headers (WHAT — here / WHAT — carried / WITH…; VERB's header
    // carries no text anymore, also SQ-0667): muted until the column holds
    // the cursor.
    row("band.column_header", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("band.column_header:active", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, false)),
    // The one-click quick-action row along the bottom of the band.
    row("band.quick", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // The quick block's hover highlight (SQ-0677): reversed video, since the
    // quick block lost its arrow-armed keyboard state (armed columns reuse
    // `dialog.list_selected` instead, a fg/bg swap, not REVERSED) and hover is
    // now its only transient highlight — the two must never look the same.
    row("band.quick:hover", Section::Elements, Kind::Style, Some("band.quick"), mods(false, false, false, true)),
    // In-column group labels and the "(nothing visible)" placeholder.
    row("band.group_label", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    // SQ-1135: a noun-column row that came from what the story PRINTED rather
    // than from its object tree. Same column, weaker claim — the story knows the
    // word, which is not a promise the thing is here — so it is dimmed. The
    // band's first per-ROW selector; every other row still takes the panel's
    // base style, and the selection (`dialog.list_selected`) still overrides.
    row("band.item:seen", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // The file browser's current-directory row and unselected directory entries.
    row("file_browser_cwd", Section::Elements, Kind::Style, Some("alert"), Delta::EMPTY),
    row("file_browser_dir", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    // The room inspector's per-edge distorted/ok rows (colorblind-hostile as
    // bare literals; kept explicit since neither has a matching role).
    row("inspector_edge_ok", Section::Elements, Kind::Style, None, fg(Color::Green)),
    row("inspector_edge_distorted", Section::Elements, Kind::Style, None, fg(Color::Red)),
    // The transcript search-match highlight (Black-on-Yellow).
    row("transcript_search_highlight", Section::Elements, Kind::Style, None,
        Delta { fg: Some(Color::Black), bg: Some(Color::Yellow), ..Delta::EMPTY }),
]);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The full set of selector names the spec (§1–§4b) requires, copied out
    /// explicitly so the test fails loudly if the registry drifts.
    const EXPECTED: &[&str] = &[
        // §1 roles
        "text",
        "chrome",
        "line",
        "accent",
        "muted",
        "alert",
        "heading",
        // §2 elements
        "transcript",
        "status_bar",
        "help_bar",
        "upper_window",
        "story_info",
        "status_header",
        "story_title",
        "input_line",
        "suggestion_line",
        "scrollbar",
        "scrollbar_track",
        "transcript_location",
        "story_badge",
        "badge_icons",
        "badge_save",
        "badge_hint",
        "badge_hint_available",
        "hyperlink",
        "story_info_label",
        "suggestion",
        "transcript_meta",
        "transcript_warning",
        "transcript_crash",
        "transcript_assist",
        "transcript_assist_caution",
        "transcript_reveal",
        // §2a/§2b panel.*
        "panel.background",
        "panel.border",
        "panel.border:active",
        "panel.title",
        "panel.tab",
        "panel.tab:active",
        "panel.tab_divider",
        "panel.terminator_left",
        "panel.terminator_right",
        "panel.control",
        "panel.control:lit",
        "panel.control:hover",
        // §3 glk.buffer.*
        "glk.buffer.normal",
        "glk.buffer.emphasized",
        "glk.buffer.preformatted",
        "glk.buffer.header",
        "glk.buffer.subheader",
        "glk.buffer.alert",
        "glk.buffer.note",
        "glk.buffer.blockquote",
        "glk.buffer.input",
        "glk.buffer.user1",
        "glk.buffer.user2",
        // §3 glk.grid.*
        "glk.grid.normal",
        "glk.grid.emphasized",
        "glk.grid.preformatted",
        "glk.grid.header",
        "glk.grid.subheader",
        "glk.grid.alert",
        "glk.grid.note",
        "glk.grid.blockquote",
        "glk.grid.input",
        "glk.grid.user1",
        "glk.grid.user2",
        // §4 map.*
        "map.background",
        "map.room",
        "map.room_current",
        "map.room_selected",
        "map.connector",
        "map.connector_distorted",
        "map.connector_portal",
        "map.shared_path",
        "map.loc_indicator",
        "map.matrix.header",
        "map.matrix.row:here",
        "map.matrix.row:selected",
        "map.matrix.cell:entrance",
        "map.matrix.cell:path",
        "map.matrix.cell:frontier",
        "map.matrix.footnote",
        "map.edge:oneway",
        "map.edge:asym",
        "map.trail",
        "map.box_style",
        "map.arrow_set",
        "map.portal_icons",
        "map.path_style",
        "map.portal_path_style",
        "map.control_icons",
        "map.diagonal_corners",
        // §4b debug.*
        "debug.pc",
        "debug.disasm_executed",
        "debug.disasm_rd",
        "debug.disasm_soft",
        "debug.disasm_data",
        "debug.zstring",
        // §2c dialog.* / §2d tooltip.*
        "dialog.background",
        "dialog.border",
        "dialog.title",
        "dialog.button",
        "dialog.button:active",
        "dialog.shadow",
        "tooltip.background",
        "tooltip.border",
        // §2 elements (expansion, Task 5.0)
        "more_prompt",
        "tidy_progress",
        "meta_marker",
        "inventory_dock",
        "room_dock",
        "room_dock.header",
        "room_dock.header:pinned",
        "story_info_title",
        "terminal_dump_heading",
        "terminal_dump_assumed",
        "story_info_value",
        "story_info_blurb",
        "story_info_link",
        "story_info_cover",
        // SQ-0861: the wrapped-continuation marker
        "story_info_continuation",
        // SQ-0789: the info panel's detected-artwork block
        "story_info_artwork",
        "story_info_artwork:active",
        "graphics",
        "inline_image",
        "story_header",
        "story_header_active",
        "story_author",
        "story_year",
        "story_rating",
        "story_no_metadata",
        "story_tile",
        "story_tile_selected",
        "story_folder",
        "notification",
        "hotkey_key",
        "sound_beep_high",
        "sound_beep_low",
        "transcript_input",
        "transcript_system",
        "warning_marker",
        "input_text",
        "input_prompt",
        "upper_window_border",
        "room_panel",
        "palette_query",
        "palette_name",
        "palette_match",
        "palette_desc",
        "palette_selected",
        "ifdb_result",
        "ifdb_result_selected",
        "ifdb_result_meta",
        "ifdb_download_marker",
        "ifdb_download_present",
        "saves_portable",
        "saves_host_only",
        "ifdb_attribution",
        // SQ-0643: hard-coded modal/list styles → themed selectors
        "dialog.list_selected",
        "dialog.list_footer",
        "dialog.list_header",
        "dialog.hint_suggestion",
        // SQ-0789: the launch-options dialog's caveat lines
        "dialog.launch_caveat",
        // SQ-0439: the region prompt
        "dialog.region_prompt.body",
        "dialog.region_prompt.rooms",
        "dialog.region_prompt.room",
        "dialog.region_prompt.option",
        "dialog.region_prompt.option:chosen",
        // SQ-1104: the font check's sample rows
        "dialog.font_check.sample",
        // SQ-0664: the command band
        "band.column_header",
        "band.column_header:active",
        "band.quick",
        "band.quick:hover",
        "band.group_label",
        // SQ-1135: the printed-word rows in the noun columns
        "band.item:seen",
        "file_browser_cwd",
        "file_browser_dir",
        "inspector_edge_ok",
        "inspector_edge_distorted",
        "transcript_search_highlight",
    ];

    #[test]
    fn expanded_elements_resolve_from_roles() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        use ratatui::style::Modifier;

        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new(), &Decls::new(), &Decls::new());

        // story_info_blurb: muted + italic — fg snaps to muted, ITALIC preserved.
        let blurb = theme.get("story_info_blurb").style;
        assert_eq!(blurb.fg, roles.muted.fg);
        assert!(blurb.add_modifier.contains(Modifier::ITALIC));

        // sound_beep_high: bespoke amber, explicit fg (not a role derivation).
        assert_eq!(theme.get("sound_beep_high").style.fg, Some(Color::Rgb(255, 180, 40)));

        // dialog.shadow: the one explicit-colour row — keeps an explicit dark-gray bg.
        assert_eq!(theme.get("dialog.shadow").style.bg, Some(Color::DarkGray));

        // story_tile_selected: accent + reversed + bold.
        let tile_selected = theme.get("story_tile_selected").style;
        assert!(tile_selected.add_modifier.contains(Modifier::REVERSED | Modifier::BOLD));
    }

    #[test]
    fn registry_is_complete_and_unique() {
        // (b) names are unique
        let mut seen = HashSet::new();
        for r in REGISTRY.iter() {
            assert!(seen.insert(r.name), "duplicate registry name: {}", r.name);
        }

        // (a) the registry names are exactly the expected set
        let expected: HashSet<&str> = EXPECTED.iter().copied().collect();
        let missing: Vec<&str> = expected.difference(&seen).copied().collect();
        let extra: Vec<&str> = seen.difference(&expected).copied().collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "registry mismatch — missing: {missing:?}, unexpected: {extra:?}"
        );

        // (c) every non-None parent names a role or another registry row
        let roles: HashSet<&str> = ROLE_NAMES.iter().copied().collect();
        for r in REGISTRY.iter() {
            if let Some(p) = r.parent {
                assert!(
                    roles.contains(p) || seen.contains(p),
                    "row {} has parent {p:?} that is neither a role nor a registry selector",
                    r.name
                );
            }
        }
    }

    // ── SQ-0643: hard-coded modal/list styles → themed selectors ─────────────

    /// The new selectors' registry DEFAULTS must reproduce exactly what the old
    /// hardcoded `Style::new()...` literals rendered, so shipping this change
    /// causes no visual change for a user with no `style.toml` override.
    #[test]
    fn sq0643_new_selector_defaults_reproduce_the_old_hardcoded_styles() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        use ratatui::style::Modifier;

        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new(), &Decls::new(), &Decls::new());

        // dialog.list_selected: old `Style::new().fg(Black).bg(Cyan).BOLD`.
        let sel = theme.get("dialog.list_selected").style;
        assert_eq!(sel.fg, Some(Color::Black));
        assert_eq!(sel.bg, Some(Color::Cyan));
        assert!(sel.add_modifier.contains(Modifier::BOLD));

        // dialog.list_footer: old `.fg(DarkGray)` (muted role default).
        assert_eq!(theme.get("dialog.list_footer").style.fg, Some(Color::DarkGray));

        // dialog.list_header: old `.fg(White).UNDERLINED` (text role = White for
        // terminal default).
        let hdr = theme.get("dialog.list_header").style;
        assert_eq!(hdr.fg, Some(Color::White));
        assert!(hdr.add_modifier.contains(Modifier::UNDERLINED));

        // dialog.hint_suggestion: old `.fg(Yellow).DIM` (alert role = Yellow).
        let hint = theme.get("dialog.hint_suggestion").style;
        assert_eq!(hint.fg, Some(Color::Yellow));
        assert!(hint.add_modifier.contains(Modifier::DIM));

        // file_browser_cwd / file_browser_dir: old `.fg(Yellow)` / `.fg(Cyan)`.
        assert_eq!(theme.get("file_browser_cwd").style.fg, Some(Color::Yellow));
        assert_eq!(theme.get("file_browser_dir").style.fg, Some(Color::Cyan));

        // inspector_edge_ok / inspector_edge_distorted: old `.fg(Green)` / `.fg(Red)`.
        assert_eq!(theme.get("inspector_edge_ok").style.fg, Some(Color::Green));
        assert_eq!(theme.get("inspector_edge_distorted").style.fg, Some(Color::Red));

        // transcript_search_highlight: old `.fg(Black).bg(Yellow)`.
        let hl = theme.get("transcript_search_highlight").style;
        assert_eq!(hl.fg, Some(Color::Black));
        assert_eq!(hl.bg, Some(Color::Yellow));
    }

    /// A `style.toml` override on each new selector must actually change its
    /// resolved style — the whole point of moving off a hardcoded literal.
    /// Falsification: before SQ-0643 these were bare `Style::new()...`
    /// constants no override could reach; this fails against that code.
    #[test]
    fn sq0643_new_selectors_are_actually_overridable() {
        use crate::theme::resolve::resolve_theme;
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse(
            "[dialog]\n\
             list_selected = { fg = \"magenta\" }\n\
             list_footer = { fg = \"magenta\" }\n\
             list_header = { fg = \"magenta\" }\n\
             hint_suggestion = { fg = \"magenta\" }\n\
             [elements]\n\
             file_browser_cwd = { fg = \"magenta\" }\n\
             file_browser_dir = { fg = \"magenta\" }\n\
             inspector_edge_ok = { fg = \"magenta\" }\n\
             inspector_edge_distorted = { fg = \"magenta\" }\n\
             transcript_search_highlight = { fg = \"magenta\" }\n",
        )
        .unwrap();
        let theme = resolve_theme(&scheme, &parsed);

        for sel in [
            "dialog.list_selected",
            "dialog.list_footer",
            "dialog.list_header",
            "dialog.hint_suggestion",
            "file_browser_cwd",
            "file_browser_dir",
            "inspector_edge_ok",
            "inspector_edge_distorted",
            "transcript_search_highlight",
        ] {
            assert_eq!(
                theme.get(sel).style.fg,
                Some(Color::Magenta),
                "selector {sel} must pick up its style.toml override"
            );
        }
    }
    /// SQ-1170: no registry row may be documented in the shipped template and
    /// read by NOTHING.
    ///
    /// `template::tests::every_key_the_template_documents_takes_effect_when_uncommented`
    /// proves a documented key is READ — parsed, lowered, resolved, and visible
    /// in the artifact a renderer draws from. It cannot prove that any renderer
    /// then draws with it, because a selector nothing looks up resolves exactly
    /// as cleanly as one everything looks up. That is how two rows shipped for
    /// months documented, parsed, resolved, warned about by nothing, and
    /// reaching no pixel: `map.background` (the map pane simply never painted a
    /// ground, where the debug pane has always painted `panel.background`) and
    /// `dialog.input_preview` (whose consumer, the verb menu's composed-input
    /// line, went with the verb menu itself).
    ///
    /// So this walks `crates/app/src` — minus `theme/`, which is the DECLARING
    /// side — and asks of each row: does anything actually name it?
    ///
    /// Three families spell the lookup differently, and each is checked by its
    /// own spelling rather than exempted from the check:
    ///
    /// - a **role** is consumed by INHERITANCE, so it is read when some registry
    ///   row parents onto it. `heading` is named in no source file and styles
    ///   two rows; requiring a literal would call it dead.
    /// - a **[`Kind::Placement`]** row is read through the SYMBOL path by its
    ///   bare key (`box_style`, `badge_save`), never by its full selector name.
    /// - the **`glk.*`** families are the app's one CONSTRUCTED selector family
    ///   (`format!("glk.{win}.{name}")` in `render::glk_theme_modifiers`), so a
    ///   row is read when its leaf is one of `GLK_STYLE_NAMES` — which means a
    ///   `glk.buffer.user3` row with no matching Glk style still fails here.
    ///
    /// One limit worth knowing: a literal inside a `#[cfg(test)]` block counts,
    /// so a row only a test names passes. Both rows this case was written for
    /// had zero references anywhere, which is the shape it catches.
    #[test]
    fn no_registry_row_is_documented_but_read_by_nothing() {
        use std::collections::HashSet;

        // Every span between two double quotes in every non-`theme/` source
        // file. Deliberately an OVER-approximation: overlapping spans mean a
        // mis-pairing (an escaped quote, a `"` inside a comment) ADDS candidates
        // rather than dropping them, so the only mistake this can make is
        // calling a row read — never calling a live row dead.
        let mut quoted: HashSet<&str> = HashSet::new();
        let mut sources: Vec<String> = Vec::new();
        let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read crates/app/src") {
                let path = entry.expect("directory entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "theme") {
                        continue;
                    }
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    sources.push(std::fs::read_to_string(&path).expect("read source file"));
                }
            }
        }
        assert!(sources.len() > 100, "only {} source files scanned — the walk is wrong", sources.len());
        for text in &sources {
            let mut open: Option<usize> = None;
            for (i, _) in text.match_indices('"') {
                if let Some(start) = open {
                    if i - start <= 64 {
                        quoted.insert(&text[start + 1..i]);
                    }
                }
                open = Some(i);
            }
        }

        for row in REGISTRY.iter() {
            let name = row.name;
            let read = if ROLE_NAMES.contains(&name) {
                REGISTRY.iter().any(|r| r.parent == Some(name))
            } else if row.kind == Kind::Placement {
                quoted.contains(name.rsplit('.').next().expect("a name has a leaf"))
            } else if let Some(leaf) =
                name.strip_prefix("glk.buffer.").or_else(|| name.strip_prefix("glk.grid."))
            {
                crate::render::GLK_STYLE_NAMES.contains(&leaf)
            } else {
                quoted.contains(name)
            };
            assert!(
                read,
                "registry row {name:?} is documented in style.example.toml and read by nothing \
                 outside crates/app/src/theme — wire it to the renderer that should honour it, \
                 or retire the row; do not leave a knob that does nothing"
            );
        }
    }
}
