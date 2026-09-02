//! The single-layer resolver: flatten [`REGISTRY`] into a concrete theme map.
//!
//! Each registry row derives from a parent (a role, or another selector) via a
//! [`Delta`]. This pass starts from the parent's resolved [`Style`], layers the
//! row's `default_delta`, then applies any matching explicit override from
//! `Decls`. The result is a flat `name -> Resolved` map queried by [`Theme::get`].
//!
//! This is the **layered** resolver (SQ-0309 Task 0.3). It applies several
//! [`Decls`] layers in the spec's static build order (registry default → global
//! user → shipped garglk.ini → per-game overlay, per-game LAST) and stamps each
//! resolved selector with the [`Provenance`] of the highest layer that supplied a
//! value for it. The stamp is **per-selector** (which layer last wrote this
//! selector name), not per-channel — sufficient for the static build order; the
//! runtime per-cell lift lands in Wave 3.

use std::collections::HashMap;

use ratatui::style::{Color, Modifier, Style};

use super::registry::{Delta, RegRow, REGISTRY, ROLE_NAMES};

/// The 7 resolved role roots (§1). Everything else derives from one of these.
///
/// Concrete colours come from the base scheme + `[roles]`; here we hold the
/// already-resolved [`Style`] per role. [`Roles::terminal_default`] provides the
/// spec's default (dark) role palette so callers/tests have a concrete input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roles {
    pub text: Style,
    pub chrome: Style,
    pub line: Style,
    pub accent: Style,
    pub muted: Style,
    pub alert: Style,
    pub heading: Style,
}

impl Roles {
    /// The spec's default (dark) role palette (design §1 / the `[roles]` example):
    /// text = white on terminal bg, chrome = white on black, line/accent = cyan,
    /// muted = dark-gray, alert = yellow, heading = white + bold.
    pub fn terminal_default() -> Roles {
        Roles {
            text: Style::default().fg(Color::White),
            chrome: Style::default().fg(Color::White).bg(Color::Black),
            line: Style::default().fg(Color::Cyan),
            accent: Style::default().fg(Color::Cyan),
            muted: Style::default().fg(Color::DarkGray),
            alert: Style::default().fg(Color::Yellow),
            heading: Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        }
    }

    /// Look up a role's [`Style`] by its name (one of [`ROLE_NAMES`]).
    fn by_name(&self, name: &str) -> Option<Style> {
        Some(match name {
            "text" => self.text,
            "chrome" => self.chrome,
            "line" => self.line,
            "accent" => self.accent,
            "muted" => self.muted,
            "alert" => self.alert,
            "heading" => self.heading,
            _ => return None,
        })
    }

    /// A mutable reference to a role's [`Style`] by name, for applying `[roles]`
    /// overrides in place. `None` for an unrecognised name.
    fn by_name_mut(&mut self, name: &str) -> Option<&mut Style> {
        Some(match name {
            "text" => &mut self.text,
            "chrome" => &mut self.chrome,
            "line" => &mut self.line,
            "accent" => &mut self.accent,
            "muted" => &mut self.muted,
            "alert" => &mut self.alert,
            "heading" => &mut self.heading,
            _ => return None,
        })
    }

    /// Derive the 7 role roots from a base colour scheme, matching today's
    /// `ColorScheme::from_ghostty` element→scheme mapping so the derived theme
    /// reproduces the current look.
    pub fn from_scheme(scheme: &crate::colors::GhosttyScheme) -> Roles {
        // No configured scheme: `resolve_base(None)` hands back a
        // `GhosttyScheme::default()` whose fg/bg/palette are all `Color::Reset`.
        // Deriving roles from that would paint every element terminal-default
        // monochrome (losing cyan borders/accents etc.). Fall back to the concrete
        // terminal-default role palette so the out-of-box look keeps its colours.
        if scheme.foreground == Color::Reset {
            return Roles::terminal_default();
        }
        let fg = scheme.foreground;
        let bg = scheme.background;
        // SQ-0642: a VALID scheme may still carry an incomplete palette (a
        // Ghostty theme only needs background+foreground; unset palette slots
        // stay `Reset`). A role derived from a Reset slot resolves every
        // dependent selector to Reset — the whole UI goes monochrome and
        // `palette:N` values vanish. Fall back PER ROLE to the terminal-default
        // role colour rather than mixing a half-resolved layer into everything.
        let fallback = Roles::terminal_default();
        let slot = |idx: usize, fb: Style| -> Style {
            if scheme.palette[idx] == Color::Reset {
                fb
            } else {
                Style::default().fg(scheme.palette[idx])
            }
        };
        Roles {
            text: Style::default().fg(fg),               // transcript = foreground
            chrome: Style::default().fg(fg).bg(bg),       // ink on a UI surface
            line: slot(6, fallback.line),   // cyan slot (focused_border/connector)
            accent: slot(6, fallback.accent), // highlight = cyan slot
            muted: slot(8, fallback.muted),   // suggestion = bright-black slot
            alert: slot(3, fallback.alert),   // yellow slot (room_selected)
            heading: Style::default().fg(fg).add_modifier(Modifier::BOLD),
        }
    }
}

/// The glyph a resolved selector carries: a gutter mark, tab divider, terminator
/// cap, or a [`Kind::Placement`](super::registry::Kind::Placement) preset name.
/// Owned so a [`Theme`] is self-contained.
///
/// There is deliberately no named-slot sub-map here. A `glyphs = { tl = "+" }`
/// table used to parse this far and then reach no renderer at all (SQ-0560);
/// individual map glyphs are overridden through `[map.overrides]`, which is the
/// one mechanism for that job.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphSet {
    pub single: Option<String>,
}

impl GlyphSet {
    fn is_empty(&self) -> bool {
        self.single.is_none()
    }
}

/// Which layer supplied a resolved selector's winning value (§5 static build
/// order). Ordered low→high: a later layer overrides an earlier one, so the
/// [`Provenance`] stamp records the *highest* layer that wrote the selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The registry default (no explicit layer touched this selector).
    Default,
    /// The global user `style.toml`.
    GlobalUser,
    /// The shipped/bundled `garglk.ini`.
    Garglk,
    /// The per-game overlay (the highest layer).
    PerGame,
}

/// A fully resolved selector: its concrete [`Style`], any glyph(s) it carries,
/// its border style (if any), and the [`Provenance`] of the layer that last set
/// its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub style: Style,
    pub glyph: Option<GlyphSet>,
    pub border: Option<crate::render::paneframe::BorderStyle>,
    /// Per-side border-style overrides (SQ-0641): a set side wins over `border`
    /// for that edge; an unset side uses `border`. Lowered from `style_top` /
    /// `style_bottom` / `style_left` / `style_right`.
    pub border_top: Option<crate::render::paneframe::BorderStyle>,
    pub border_bottom: Option<crate::render::paneframe::BorderStyle>,
    pub border_left: Option<crate::render::paneframe::BorderStyle>,
    pub border_right: Option<crate::render::paneframe::BorderStyle>,
    /// Pane header-strip toggle (`header = true/false`); `None` = consumer default.
    pub header: Option<bool>,
    /// Drop-shadow toggle (`shadow = true/false`); `None` = consumer default.
    pub shadow: Option<bool>,
    pub provenance: Provenance,
}

impl Resolved {
    /// A bare resolution: just a style, nothing else set.
    fn bare(style: Style) -> Resolved {
        Resolved {
            style,
            glyph: None,
            border: None,
            border_top: None,
            border_bottom: None,
            border_left: None,
            border_right: None,
            header: None,
            shadow: None,
            provenance: Provenance::Default,
        }
    }
}

/// Explicit per-selector overrides for a single layer, on top of the registry
/// default. [`resolve`] stacks several of these (global / garglk / per-game).
pub type Decls = HashMap<String, Delta>;

/// The flat resolved theme: `selector name -> Resolved`.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    map: HashMap<String, Resolved>,
    /// Fallback for an unknown selector — the `text` role.
    fallback: Resolved,
    /// Non-fatal resolution diagnostics (SQ-0642): e.g. a `parent` re-root
    /// naming an unknown selector or forming a cycle — the offending rows fell
    /// back to their registry parents. Carried on the theme because the
    /// resolvers have no warnings channel of their own (style-load warnings are
    /// returned by `style::resolve`, which does not build the layered theme).
    warnings: Vec<ThemeWarning>,
}

impl Theme {
    /// Resolve a selector. An unknown selector falls back to the `text` role
    /// (the body-ink default) so a stray name never panics or renders unstyled.
    pub fn get(&self, sel: &str) -> Resolved {
        self.map.get(sel).cloned().unwrap_or_else(|| self.fallback.clone())
    }

    /// Is `sel` still nobody's — neither it nor the `role` it inherits from
    /// touched by any layer?
    ///
    /// A machine default may fill an unclaimed selector and must not overwrite a
    /// CHOICE, which is the rule SQ-0847 established for the Macintosh's white
    /// page and SQ-0873's period look reuses. [`Provenance`] is stamped per row
    /// and does not travel down the parent chain, so a player who recoloured the
    /// `text` role and left `transcript` alone has still chosen the transcript's
    /// ink — hence the second argument. Pass the row's registry parent.
    pub fn unclaimed(&self, sel: &str, role: &str) -> bool {
        [sel, role].iter().all(|s| {
            self.map.get(*s).map(|r| r.provenance == Provenance::Default).unwrap_or(true)
        })
    }

    /// Patch `style`'s colours into `sel`, if [`Theme::unclaimed`].
    ///
    /// A patch, so a style that sets only a background leaves the ink alone —
    /// which is what the transcript's own meta and warning selectors want from a
    /// machine's page, having ink of their own that says something lanthorn
    /// means. Modifiers on `style` are added to whatever the row already carries.
    pub fn fill_unclaimed(&mut self, sel: &str, role: &str, style: Style) {
        if !self.unclaimed(sel, role) {
            return;
        }
        if let Some(row) = self.map.get_mut(sel) {
            row.style = row.style.patch(style);
        }
    }

    /// Replace `sel`'s style outright, if [`Theme::unclaimed`].
    ///
    /// For the one selector whose registry default is itself a *rendering* rather
    /// than a colour: `status_bar` ships REVERSED, which is lanthorn's way of
    /// setting the bar apart. A machine that states its own band has already said
    /// how it is set apart — and a swapped pair drawn under a REVERSED modifier
    /// swaps back, which is a full reverse rendered as no reverse at all. So the
    /// band is stated absolutely and the row's own modifiers go with it.
    pub fn set_unclaimed(&mut self, sel: &str, role: &str, style: Style) {
        if !self.unclaimed(sel, role) {
            return;
        }
        if let Some(row) = self.map.get_mut(sel) {
            row.style = style;
        }
    }

    /// Non-fatal diagnostics from the resolution pass (empty when clean).
    pub fn warnings(&self) -> &[ThemeWarning] {
        &self.warnings
    }
}

/// Whether a rejected `parent` re-root named something that does not exist at
/// all (a typo) or named a real selector/role that only fails to resolve
/// because it takes part in a re-root cycle (SQ-0663). Distinguishing the two
/// makes the surfaced message say something more useful than "unknown or
/// forms a cycle" for every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeWarningKind {
    /// `parent` names no role and no registry selector.
    UnknownParent,
    /// `parent` names a real role/selector, but re-rooting onto it cycles.
    Cycle,
}

/// A non-fatal theme-resolution diagnostic (SQ-0642/SQ-0663): a selector's
/// `parent` override was rejected — the name doesn't resolve to anything, or
/// re-rooting onto it forms a cycle — so the selector fell back to its
/// registry-default parent instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeWarning {
    /// The selector whose `parent` override was rejected, e.g. `"panel.title"`.
    pub selector: String,
    /// The parent name the user wrote, e.g. `"acent"`.
    pub parent: String,
    pub kind: ThemeWarningKind,
}

impl ThemeWarning {
    /// One human-readable line for the transcript/startup surface (SQ-0663),
    /// e.g. `style.toml: [panel.title] parent = "acent" does not name a
    /// selector — using defaults for it`. Not attributed to a specific file:
    /// the resolver sees only already-lowered `Decls`/`ParsedStyle` layers, not
    /// the paths they came from (global style.toml, a discovered garglk.ini, or
    /// the per-game sidecar all fold into one resolve() pass) — see
    /// `describe_theme_warnings`'s doc comment for the same caveat.
    pub fn describe(&self) -> String {
        let reason = match self.kind {
            ThemeWarningKind::UnknownParent => "does not name a selector",
            ThemeWarningKind::Cycle => "forms a parent cycle",
        };
        format!(
            "style.toml: [{}] parent = \"{}\" {reason} — using defaults for it",
            self.selector, self.parent,
        )
    }
}

/// How many [`ThemeWarning`]s to print individually before collapsing to one
/// summarizing line (SQ-0663): a single bad base selector that many rows
/// re-root onto (or re-root onto each other) could otherwise spam the
/// transcript with one line per affected selector.
const WARNING_LINES_MAX: usize = 4;
/// How many offending selectors to name in the summarized line.
const WARNING_NAMED_MAX: usize = 3;

/// Render a theme's resolution warnings as the notice lines shown at startup
/// and on a live style reload (SQ-0663): empty input yields no lines (a clean
/// resolve stays silent — nothing is re-announced); up to [`WARNING_LINES_MAX`]
/// warnings each get their own [`ThemeWarning::describe`] line; beyond that,
/// one line names the first [`WARNING_NAMED_MAX`] offending selectors plus a
/// total count.
///
/// Per-file/per-layer attribution (global `style.toml` vs. a discovered
/// `garglk.ini` vs. a per-game sidecar) is deliberately not attempted here:
/// [`resolve_theme_layered`] folds all three `Decls`/`ParsedStyle` layers into
/// one `resolve()` pass and the warning only records the selector/parent
/// names, not which layer's `parent` override was the one that lost the
/// fixpoint race. Adding that would mean threading path/layer context through
/// the resolver just for diagnostics, so every line reads `style.toml:` (the
/// generic "your styling" prefix) rather than naming a specific file.
pub fn describe_theme_warnings(warnings: &[ThemeWarning]) -> Vec<String> {
    if warnings.is_empty() {
        return Vec::new();
    }
    if warnings.len() <= WARNING_LINES_MAX {
        return warnings.iter().map(ThemeWarning::describe).collect();
    }
    let named: Vec<&str> = warnings.iter().take(WARNING_NAMED_MAX).map(|w| w.selector.as_str()).collect();
    vec![format!(
        "style.toml: {} selectors have an invalid `parent` override ({}, …) — using defaults for them",
        warnings.len(),
        named.join(", "),
    )]
}

/// Layer a [`Delta`] onto a base [`Style`]: override fg/bg only where the delta
/// sets them; add (never clear) the delta's modifier bits.
fn apply_style(base: Style, d: &Delta) -> Style {
    apply_style_channels(base, d, true)
}

/// [`apply_style`], but with the colour channels skippable (SQ-1169): when
/// `apply_color` is `false`, `d.fg`/`d.bg` are ignored entirely and the base's
/// own colours pass through untouched — modifiers still layer as normal. The
/// registry-default step in [`resolve_row`] uses this to let a user `parent`
/// re-root actually move a row's colours: a pinned `fg`/`bg` in the registry
/// [`Delta`] applied on top of the NEW parent would otherwise silently restore
/// the old pin, exactly as it did before this selector could be re-rooted at all.
fn apply_style_channels(base: Style, d: &Delta, apply_color: bool) -> Style {
    let mut s = base;
    if apply_color {
        if let Some(fg) = d.fg {
            s = s.fg(fg);
        }
        if let Some(bg) = d.bg {
            s = s.bg(bg);
        }
    }
    // Modifiers are tri-state (SQ-1171): `None` inherits, `Some(true)` adds,
    // `Some(false)` REMOVES. Add and remove are accumulated separately and the
    // removal is applied last, so a single Delta that both sets and clears is
    // unambiguous rather than order-dependent.
    let mut m = Modifier::empty();
    let mut clear = Modifier::empty();
    for (flag, bit) in [
        (d.bold, Modifier::BOLD),
        (d.italic, Modifier::ITALIC),
        (d.underline, Modifier::UNDERLINED),
        (d.reversed, Modifier::REVERSED),
        (d.dim, Modifier::DIM),
    ] {
        match flag {
            Some(true) => m |= bit,
            Some(false) => clear |= bit,
            None => {}
        }
    }
    if !clear.is_empty() {
        s = s.remove_modifier(clear);
    }
    if !m.is_empty() {
        s = s.add_modifier(m);
    }
    s
}

/// Layer a [`Delta`]'s glyph channel onto an inherited [`GlyphSet`]: a set glyph
/// overrides the inherited one; otherwise it carries through.
fn apply_glyph(inherited: Option<GlyphSet>, d: &Delta) -> Option<GlyphSet> {
    let mut g = inherited.unwrap_or_default();
    if let Some(single) = &d.glyph {
        g.single = Some(single.clone());
    }
    if g.is_empty() {
        None
    } else {
        Some(g)
    }
}

/// Layer a [`Delta`]'s border-style onto the inherited one: a set preset name
/// overrides (parsed via the paneframe grammar); otherwise it carries through.
fn apply_border(
    inherited: Option<crate::render::paneframe::BorderStyle>,
    d: &Delta,
) -> Option<crate::render::paneframe::BorderStyle> {
    d.border.or(inherited)
}

/// Resolve one row against a known parent `Resolved` (or a bare parent style when
/// the row has no parent): the registry default on the parent, then each explicit
/// `layers` override in build order (lowest→highest). Each layer that carries a
/// [`Delta`] for this selector overrides the running value AND advances the
/// [`Provenance`] stamp, so the stamp reflects the highest layer that wrote it.
///
/// `rerooted` (SQ-1169) is whether some USER layer set a `parent` override for
/// this row (see [`user_rerooted`]) — `parent` here is already that new parent's
/// `Resolved`. When `true`, the registry default delta's `fg`/`bg` are skipped so
/// the new parent's colours reach the row; a row whose default delta pins a
/// colour would otherwise silently restore the old parent's pin on top of the
/// re-root, making `parent = "…"` a no-op (or a half-op, for a delta pinning only
/// one channel). The delta's non-colour channels (modifiers, glyph, border, …)
/// are unaffected — a re-root moves ink, not weight or shape.
fn resolve_row(row: &RegRow, parent: &Resolved, layers: &[(&Decls, Provenance)], rerooted: bool) -> Resolved {
    // 1. registry default delta on the parent.
    let d = &row.default_delta;
    let mut style = apply_style_channels(parent.style, d, !rerooted);
    let mut glyph = apply_glyph(parent.glyph.clone(), d);
    let mut border = apply_border(parent.border, d);
    // Structural channels (SQ-0641): same set-wins-else-inherit rule as border.
    let mut border_top = d.border_top.or(parent.border_top);
    let mut border_bottom = d.border_bottom.or(parent.border_bottom);
    let mut border_left = d.border_left.or(parent.border_left);
    let mut border_right = d.border_right.or(parent.border_right);
    let mut header = d.header.or(parent.header);
    let mut shadow = d.shadow.or(parent.shadow);
    // 2. each explicit layer, lowest→highest; the last to touch wins the stamp.
    let mut provenance = Provenance::Default;
    for (decls, prov) in layers {
        if let Some(over) = decls.get(row.name) {
            style = apply_style(style, over);
            glyph = apply_glyph(glyph, over);
            border = apply_border(border, over);
            border_top = over.border_top.or(border_top);
            border_bottom = over.border_bottom.or(border_bottom);
            border_left = over.border_left.or(border_left);
            border_right = over.border_right.or(border_right);
            header = over.header.or(header);
            shadow = over.shadow.or(shadow);
            provenance = *prov;
        }
    }
    Resolved {
        style,
        glyph,
        border,
        border_top,
        border_bottom,
        border_left,
        border_right,
        header,
        shadow,
        provenance,
    }
}

/// The effective parent selector for a row: a user layer's `parent` override
/// (SQ-0440) wins over the registry default, taking the highest layer that sets
/// one. `None` means a bare root (no inheritance). Returns an owned name because a
/// user override is runtime-owned while the registry default is `&'static`.
fn effective_parent(row: &RegRow, layers: &[(&Decls, Provenance)]) -> Option<String> {
    let mut parent = row.parent.map(str::to_string);
    for (decls, _) in layers {
        if let Some(p) = decls.get(row.name).and_then(|d| d.parent.as_ref()) {
            parent = Some(p.clone());
        }
    }
    parent
}

/// Whether any user layer's decl for `row` sets `parent` at all (SQ-1169) —
/// intent, not value: a re-root counts even when it names the row's own
/// registry-default parent, because the point is that the USER chose it. Kept
/// separate from [`effective_parent`], which resolves to a name rather than a
/// yes/no, so [`resolve_row`] can tell "re-rooted onto the same name" apart
/// from "no layer touched `parent` at all" — the two must not collapse, or a
/// row's pinned colours would survive a re-root that happens to restate the
/// default parent.
fn user_rerooted(row: &RegRow, layers: &[(&Decls, Provenance)]) -> bool {
    layers.iter().any(|(decls, _)| decls.get(row.name).is_some_and(|d| d.parent.is_some()))
}

/// Compute the flat theme map from the registry via single-level parent fallback.
///
/// Roles resolve first (from `roles`); then each row resolves against its parent —
/// a role, or another already-resolved selector. A parent that is another selector
/// is handled generally: rows resolve in dependency order via a fixpoint loop, so a
/// parent is always resolved before its child (the registry currently only parents
/// roles, for which one pass suffices).
pub fn resolve(roles: &Roles, global: &Decls, garglk: &Decls, per_game: &Decls) -> Theme {
    // The layers in the spec's static build order (lowest → highest); per-game LAST.
    let layers: [(&Decls, Provenance); 3] = [
        (global, Provenance::GlobalUser),
        (garglk, Provenance::Garglk),
        (per_game, Provenance::PerGame),
    ];

    let mut map: HashMap<String, Resolved> = HashMap::new();
    let mut warnings: Vec<ThemeWarning> = Vec::new();

    // Roles first: their Resolved is the bare role style (no glyph, no delta).
    for name in ROLE_NAMES {
        let style = roles.by_name(name).expect("ROLE_NAMES entry has a role style");
        // A role row may still carry an explicit override.
        let row = REGISTRY.iter().find(|r| r.name == name);
        let resolved = match row {
            // Roles are roots: they never consult `parent` (there is nothing to
            // re-root onto), so `rerooted` is always `false` here.
            Some(r) => resolve_row(r, &Resolved::bare(style), &layers, false),
            None => Resolved::bare(style),
        };
        map.insert(name.to_string(), resolved);
    }

    // Everything else: resolve in dependency order. A row is resolvable once its
    // parent is a role (already in `map`), None (a bare root), or another selector
    // already resolved. Loop to a fixpoint so selector->selector parents work.
    let mut pending: Vec<&RegRow> =
        REGISTRY.iter().filter(|r| !ROLE_NAMES.contains(&r.name)).collect();

    loop {
        let before = pending.len();
        pending.retain(|row| {
            let parent = match effective_parent(row, &layers) {
                None => Resolved::bare(Style::default()),
                Some(p) => match map.get(&p) {
                    Some(res) => res.clone(),
                    None => return true, // parent not resolved yet; keep pending.
                },
            };
            let rerooted = user_rerooted(row, &layers);
            let resolved = resolve_row(row, &parent, &layers, rerooted);
            map.insert(row.name.to_string(), resolved);
            false
        });
        if pending.is_empty() {
            break;
        }
        if pending.len() == before {
            // No progress: a user `parent` re-root names a selector that does
            // not exist (a typo) or forms a cycle — the registry test
            // guarantees REGISTRY parents themselves always exist. SQ-0642:
            // the remainder used to resolve against an empty `Style::default()`
            // root, stripping even the row's registry-default styling. Fall
            // back to each row's REGISTRY parent instead (ignoring the bad
            // user re-root) and surface a warning.
            for row in pending.drain(..) {
                let user_parent = effective_parent(row, &layers);
                let registry_parent = row.parent.map(str::to_string);
                if user_parent != registry_parent {
                    // `user_parent` is always `Some` here: it only differs from
                    // `registry_parent` when some layer's `Decls` actually set a
                    // `parent` override for this row (see `effective_parent`).
                    let bad_parent = user_parent.clone().unwrap_or_default();
                    let known = ROLE_NAMES.contains(&bad_parent.as_str())
                        || REGISTRY.iter().any(|r| r.name == bad_parent);
                    warnings.push(ThemeWarning {
                        selector: row.name.to_string(),
                        parent: bad_parent,
                        kind: if known { ThemeWarningKind::Cycle } else { ThemeWarningKind::UnknownParent },
                    });
                }
                let parent = registry_parent
                    .and_then(|p| map.get(&p).cloned())
                    .unwrap_or_else(|| Resolved::bare(Style::default()));
                // The bad re-root is being IGNORED here (falling back to the
                // REGISTRY parent, not the user's), so this is not a re-root as
                // far as the registry default delta is concerned.
                let resolved = resolve_row(row, &parent, &layers, false);
                map.insert(row.name.to_string(), resolved);
            }
            break;
        }
    }

    let fallback = map.get("text").cloned().expect("text role is always resolved");
    Theme { map, fallback, warnings }
}

/// Build the flat [`Theme`] from a base colour `scheme` and a parsed style document.
///
/// Roles start from [`Roles::from_scheme`] and are overridden by `parsed.roles`
/// (fg/bg only — roles carry no user modifiers today, aside from `heading`'s
/// already-baked-in bold). `parsed.decls` lower to a single GLOBAL [`Decls`]
/// layer via [`lower_decls`], which now carries fg/bg/modifiers plus the border
/// `style`, `glyph`/`glyphs`, and `parent` re-root (SQ-0440). Colour strings
/// resolve against `scheme` via [`crate::colors::parse_color_value`].
///
/// Still deferred: role modifiers and modifier-clearing (an unset/`false` flag is
/// a no-op, matching [`apply_style`], so a user cannot yet CLEAR a default modifier).
pub fn resolve_theme(
    scheme: &crate::colors::GhosttyScheme,
    parsed: &super::toml_schema::ParsedStyle,
) -> Theme {
    resolve_theme_layered(scheme, parsed, &Decls::new(), &super::toml_schema::ParsedStyle::default())
}

/// Build the flat [`Theme`] from a base `scheme` and TWO parsed layers: the global
/// user `style.toml` and the per-game overlay (per-game wins, §5). Roles start from
/// [`Roles::from_scheme`], then global `[roles]` fg/bg overrides, then per-game
/// `[roles]` overrides. Each layer's `decls` lower to its own [`Decls`] (fg/bg +
/// modifiers) so [`resolve`] stamps [`Provenance`] per selector (GlobalUser / PerGame).
/// The `garglk` [`Decls`] layer (a discovered garglk.ini's colour overlay, built by
/// `garglk_ini::garglk_color_decls`) sits between global and per-game (§5 order).
///
/// Decl `parent` re-rooting and `glyph`/`glyphs`/border overrides now all flow
/// through (SQ-0440). Still deferred: role modifiers and modifier-clearing.
pub fn resolve_theme_layered(
    scheme: &crate::colors::GhosttyScheme,
    global: &super::toml_schema::ParsedStyle,
    garglk: &Decls,
    per_game: &super::toml_schema::ParsedStyle,
) -> Theme {
    // 1. base roles from scheme, then [roles] fg/bg overrides (global, then per-game).
    let mut roles = Roles::from_scheme(scheme);
    let global_role_decls = lower_role_decls(&global.roles, scheme);
    let per_game_role_decls = lower_role_decls(&per_game.roles, scheme);
    apply_role_overrides(&mut roles, &global_role_decls);
    apply_role_overrides(&mut roles, &per_game_role_decls);

    // 2. lower each layer's decls -> its own Decls layer. The [roles] fg/bg
    //    overrides join the same layer (keyed by role name, which is itself a
    //    REGISTRY row name) so the role selector's own Provenance is stamped
    //    consistently with every other selector, via the same `resolve` pass.
    let mut global_decls = lower_decls(&global.decls, scheme);
    global_decls.extend(global_role_decls);
    let mut per_game_decls = lower_decls(&per_game.decls, scheme);
    per_game_decls.extend(per_game_role_decls);

    resolve(&roles, &global_decls, garglk, &per_game_decls)
}

/// Apply a layer's already-lowered `[roles]` fg/bg [`Decls`] onto `roles` in
/// place (so dependent selectors inherit the override via their parent role).
/// Unrecognised role names are ignored. Modifiers are not applied (Wave 5).
fn apply_role_overrides(roles: &mut Roles, role_decls: &Decls) {
    for (name, delta) in role_decls {
        let Some(style) = roles.by_name_mut(name) else { continue };
        *style = apply_style(*style, delta);
    }
}

/// Lower a layer's raw `[roles]` overrides to a [`Decls`] map of fg/bg-only
/// [`Delta`]s (role modifiers are Wave 5 — a role entry with neither fg nor bg
/// set is dropped). Shared by [`apply_role_overrides`] (role-derived selectors)
/// and [`resolve_theme_layered`]'s own decls layer (the role selector's stamp).
fn lower_role_decls(
    raw_roles: &std::collections::BTreeMap<String, super::toml_schema::RawDelta>,
    scheme: &crate::colors::GhosttyScheme,
) -> Decls {
    use crate::colors::parse_color_value;

    let mut decls = Decls::new();
    for (name, raw) in raw_roles {
        let fg = raw.fg.as_deref().and_then(|s| parse_color_value(s, scheme));
        let bg = raw.bg.as_deref().and_then(|s| parse_color_value(s, scheme));
        if fg.is_none() && bg.is_none() {
            continue;
        }
        decls.insert(name.clone(), Delta { fg, bg, ..Delta::EMPTY });
    }
    decls
}

/// Lower a layer's raw `decls` to a [`Decls`] map of resolved [`Delta`]s. Colour
/// strings resolve against `scheme` via [`crate::colors::parse_color_value`].
///
/// All override channels flow through (SQ-0440): fg/bg/modifiers, the border
/// `style`, `glyph`/`glyphs`, and a `parent` re-root (applied in [`resolve`] via
/// [`effective_parent`]). Modifiers are additive only (an unset/`false` flag is a
/// no-op, matching [`apply_style`]) — a user cannot yet CLEAR a default modifier.
fn lower_decls(
    raw_decls: &std::collections::BTreeMap<String, super::toml_schema::RawDelta>,
    scheme: &crate::colors::GhosttyScheme,
) -> Decls {
    use crate::colors::parse_color_value;

    let mut decls = Decls::new();
    for (name, raw) in raw_decls {
        let side = |s: &Option<String>| {
            s.as_deref().map(crate::render::paneframe::parse_border_style)
        };
        let delta = Delta {
            fg: raw.fg.as_deref().and_then(|s| parse_color_value(s, scheme)),
            bg: raw.bg.as_deref().and_then(|s| parse_color_value(s, scheme)),
            // SQ-1171: carried straight through, not flattened. `RawDelta` has
            // always parsed these as `Option<bool>` — the TOML layer could tell
            // an ABSENT key from an explicit `false` the whole time — and an
            // `unwrap_or(false)` here threw that answer away, which is why
            // `bold = false` in a user's style.toml was a silent no-op instead
            // of switching the weight off.
            bold: raw.bold,
            italic: raw.italic,
            underline: raw.underline,
            reversed: raw.reversed,
            dim: raw.dim,
            // SQ-0440: glyph / border-style / parent overrides flow through (the
            // registry `Delta` owns these channels). There is no glyph-SLOT channel:
            // a `glyphs = { … }` sub-map reached no renderer, so it is gone (SQ-0560).
            glyph: raw.glyph.clone(),
            border: raw.style.as_deref().map(crate::render::paneframe::parse_border_style),
            // SQ-0641: the per-side border styles and the header/shadow toggles
            // are declared by the schema (toml_schema::RawDelta) and used to be
            // dropped right here, so `style_top` / `header` / `shadow` parsed
            // and did nothing.
            border_top: side(&raw.style_top),
            border_bottom: side(&raw.style_bottom),
            border_left: side(&raw.style_left),
            border_right: side(&raw.style_right),
            header: raw.header,
            shadow: raw.shadow,
            parent: raw.parent.clone(),
        };
        decls.insert(name.clone(), delta);
    }
    decls
}

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;

    /// Resolve with no explicit layers — the registry-default theme.
    fn resolve_default(roles: &Roles) -> Theme {
        resolve(roles, &Decls::new(), &Decls::new(), &Decls::new())
    }

    /// A one-entry [`Decls`] for `sel` carrying `delta`.
    fn one(sel: &str, delta: Delta) -> Decls {
        let mut d = Decls::new();
        d.insert(sel.to_string(), delta);
        d
    }

    #[test]
    fn unset_selector_inherits_its_parent_role() {
        let roles = Roles::terminal_default();
        let theme = resolve_default(&roles);

        // §2: `transcript` has no delta, so it IS the `text` role.
        assert_eq!(theme.get("transcript").style, roles.text);

        // §3: `glk.buffer.header` = heading role + bold. Heading is already bold,
        // so fg matches heading and BOLD is set.
        let header = theme.get("glk.buffer.header").style;
        assert_eq!(header.fg, roles.heading.fg);
        assert!(header.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn glk_buffer_emphasized_is_italic() {
        // §3 canonical defaults: Emphasized = base role + italic.
        let roles = Roles::terminal_default();
        let theme = resolve_default(&roles);

        let emph = theme.get("glk.buffer.emphasized").style;
        assert_eq!(emph.fg, roles.text.fg); // buffer base = text
        assert!(emph.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn explicit_decl_overrides_default() {
        let roles = Roles::terminal_default();

        // Without a decl, `transcript` is the text role (white fg).
        let plain = resolve_default(&roles);
        assert_eq!(plain.get("transcript").style.fg, Some(Color::White));

        // An explicit override wins over the registry default.
        let decls = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let themed = resolve(&roles, &decls, &Decls::new(), &Decls::new());
        assert_eq!(themed.get("transcript").style.fg, Some(Color::Red));
    }

    #[test]
    fn glyph_carries_from_the_default_delta() {
        // A selector whose default delta carries a glyph exposes it in Resolved.
        let theme = resolve_default(&Roles::terminal_default());
        let meta = theme.get("transcript_meta");
        assert_eq!(meta.glyph.and_then(|g| g.single), Some("▏".to_string()));
    }

    #[test]
    fn unknown_selector_falls_back_to_text() {
        let roles = Roles::terminal_default();
        let theme = resolve_default(&roles);
        assert_eq!(theme.get("no.such.selector").style, roles.text);
    }

    #[test]
    fn panel_border_resolves_single() {
        let theme = resolve_default(&Roles::terminal_default());
        assert_eq!(
            theme.get("panel.border").border,
            Some(crate::render::paneframe::BorderStyle::Single)
        );
    }

    #[test]
    fn panel_border_active_is_single_and_bold() {
        let theme = resolve_default(&Roles::terminal_default());
        let active = theme.get("panel.border:active");
        assert_eq!(active.border, Some(crate::render::paneframe::BorderStyle::Single));
        assert!(active.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn non_border_selector_has_no_border() {
        let theme = resolve_default(&Roles::terminal_default());
        assert_eq!(theme.get("transcript").border, None);
    }

    // ── SQ-0440: user border / glyph / parent overrides flow through lower_decls ──

    #[test]
    fn user_border_style_override_resolves() {
        // The reported bug: `[panel] border = { style = "double" }` was parsed but
        // dropped, so panels always rendered single. It must now resolve to Double.
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse("[panel]\nborder = { style = \"double\" }\n").unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        assert_eq!(
            theme.get("panel.border").border,
            Some(crate::render::paneframe::BorderStyle::Double),
        );
    }

    #[test]
    fn user_glyph_override_resolves() {
        // A user glyph override on a selector that carries a default glyph replaces it.
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse("[panel]\ntab_divider = { glyph = \"┃\" }\n").unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        assert_eq!(
            theme.get("panel.tab_divider").glyph.and_then(|g| g.single),
            Some("┃".to_string()),
        );
    }

    // ── SQ-0560: the map selectors' dead `glyphs` sub-map ────────────────────

    #[test]
    fn map_selectors_do_not_carry_a_glyph_slot_submap() {
        // `glyphs = { tl = "+" }` on map.room / map.connector used to parse all
        // the way into the Theme and then go nowhere: render/map.rs reads only
        // `.style` and draws from `state.symbols`. Silently accepting input that
        // reaches no renderer is the defect — `[map.overrides]` is the one
        // mechanism for a single map glyph — so the sub-map is no longer part of
        // the schema at all, on ANY selector.
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse(
            "[map]\n\
             room = { fg = \"white\", glyphs = { tl = \"+\" } }\n\
             connector = { fg = \"cyan\", glyphs = { north = \"^\" } }\n",
        )
        .unwrap();
        let theme = resolve_theme(&scheme, &parsed);

        // The colour on the same line still lands — only the glyph slots are gone.
        assert_eq!(theme.get("map.room").style.fg, Some(Color::White));
        assert_eq!(theme.get("map.connector").style.fg, Some(Color::Cyan));
        assert!(
            theme.get("map.room").glyph.is_none(),
            "map.room must carry no glyph channel from a `glyphs` sub-map"
        );
        assert!(
            theme.get("map.connector").glyph.is_none(),
            "map.connector must carry no glyph channel from a `glyphs` sub-map"
        );
    }

    #[test]
    fn single_glyph_overrides_are_untouched_by_the_slot_removal() {
        // The `glyph = "…"` channel is live and load-bearing — panel terminator
        // caps and tab dividers, the dialog/saves markers, the debug gutter
        // tiers all read it. Dropping the slot sub-map must not graze it.
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse(
            "[panel]\n\
             tab_divider = { glyph = \"┃\" }\n\
             terminator_left = { glyph = \"«\" }\n\
             [debug]\n\
             disasm_soft = { glyph = \"?\" }\n\
             [elements]\n\
             saves_portable = { glyph = \"»\" }\n",
        )
        .unwrap();
        let theme = resolve_theme(&scheme, &parsed);

        let single = |sel: &str| theme.get(sel).glyph.and_then(|g| g.single);
        assert_eq!(single("panel.tab_divider"), Some("┃".to_string()));
        assert_eq!(single("panel.terminator_left"), Some("«".to_string()));
        assert_eq!(single("debug.disasm_soft"), Some("?".to_string()));
        assert_eq!(single("saves_portable"), Some("»".to_string()));

        // …and the registry defaults for the untouched ones still resolve.
        let plain = resolve_theme(&scheme, &super::super::toml_schema::ParsedStyle::default());
        assert_eq!(
            plain.get("panel.terminator_right").glyph.and_then(|g| g.single),
            Some("├".to_string())
        );
        assert_eq!(
            plain.get("transcript_warning").glyph.and_then(|g| g.single),
            Some("!".to_string())
        );
        assert_eq!(
            plain.get("debug.disasm_executed").glyph.and_then(|g| g.single),
            Some("|".to_string())
        );
    }

    // ── SQ-0641: per-side border styles + header/shadow lower through ─────────

    #[test]
    fn dropped_structural_channels_now_lower_into_the_theme() {
        // style_top / header / shadow are declared by the schema (RawDelta) and
        // used to be dropped by lower_decls, so they parsed and did nothing.
        use crate::render::paneframe::BorderStyle;
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse(
            "[panel]\n\
             border = { style_top = \"double\", style_left = \"none\", header = false }\n\
             [dialog]\n\
             border = { shadow = true }\n",
        )
        .unwrap();
        let theme = resolve_theme(&scheme, &parsed);

        let pb = theme.get("panel.border");
        assert_eq!(pb.border_top, Some(BorderStyle::Double), "style_top must lower");
        assert_eq!(pb.border_left, Some(BorderStyle::None), "style_left must lower");
        assert_eq!(pb.border_bottom, None, "unset side stays inherit-from-border");
        assert_eq!(pb.border, Some(BorderStyle::Single), "registry border default intact");
        assert_eq!(pb.header, Some(false), "header toggle must lower");
        assert_eq!(theme.get("dialog.border").shadow, Some(true), "shadow toggle must lower");

        // Untouched selectors carry no structural overrides.
        let plain = theme.get("transcript");
        assert_eq!(plain.border_top, None);
        assert_eq!(plain.header, None);
        assert_eq!(plain.shadow, None);
    }

    // ── SQ-0642: a bad `parent` re-root must not strip registry defaults ──────

    #[test]
    fn unknown_parent_reroot_falls_back_to_the_registry_default() {
        use crate::render::paneframe::BorderStyle;
        let scheme = terminal_default_scheme();
        // "acent" is a typo for "accent": no such selector exists.
        let parsed =
            super::super::toml_schema::parse("[panel]\ntitle = { parent = \"acent\" }\n").unwrap();
        let theme = resolve_theme(&scheme, &parsed);

        // panel.title keeps its registry default (heading: white + bold), not an
        // empty Style::default() root.
        let title = theme.get("panel.title");
        assert_eq!(title.style.fg, theme.get("heading").style.fg);
        assert!(title.style.add_modifier.contains(Modifier::BOLD), "registry default styling survives");
        // The rest of the theme is untouched by the stall.
        assert_eq!(theme.get("panel.border").border, Some(BorderStyle::Single));
        // And the typo is surfaced as a diagnostic.
        assert!(
            theme.warnings().iter().any(|w| w.selector == "panel.title"
                && w.parent == "acent"
                && w.kind == ThemeWarningKind::UnknownParent),
            "warning names the offending selector: {:?}",
            theme.warnings()
        );
        assert_eq!(
            theme.warnings()[0].describe(),
            "style.toml: [panel.title] parent = \"acent\" does not name a selector — using defaults for it",
        );
    }

    #[test]
    fn parent_cycle_resolves_to_registry_defaults_without_hanging() {
        let scheme = terminal_default_scheme();
        // transcript ⇄ suggestion: a two-selector re-root cycle.
        let parsed = super::super::toml_schema::parse(
            "[elements]\n\
             transcript = { parent = \"suggestion\" }\n\
             suggestion = { parent = \"transcript\" }\n",
        )
        .unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        // Both fall back to their registry parents: transcript→text (white),
        // suggestion→muted (dark-gray) — not Style::default().
        assert_eq!(theme.get("transcript").style.fg, Some(Color::White));
        assert_eq!(theme.get("suggestion").style.fg, Some(Color::DarkGray));
        assert_eq!(theme.warnings().len(), 2, "{:?}", theme.warnings());
        assert!(
            theme.warnings().iter().all(|w| w.kind == ThemeWarningKind::Cycle),
            "both re-roots name real (existing) selectors, so this is a cycle, not a typo: {:?}",
            theme.warnings()
        );
    }

    #[test]
    fn clean_resolution_carries_no_warnings() {
        let scheme = terminal_default_scheme();
        let theme = resolve_theme(&scheme, &ParsedStyle::default());
        assert!(theme.warnings().is_empty(), "{:?}", theme.warnings());
    }

    // ── SQ-0663: describe_theme_warnings (startup/reload notice formatting) ──

    #[test]
    fn describe_theme_warnings_empty_is_silent() {
        assert!(describe_theme_warnings(&[]).is_empty());
    }

    #[test]
    fn describe_theme_warnings_one_line_per_warning_under_the_threshold() {
        let warnings = vec![
            ThemeWarning { selector: "panel.title".into(), parent: "acent".into(), kind: ThemeWarningKind::UnknownParent },
            ThemeWarning { selector: "transcript".into(), parent: "suggestion".into(), kind: ThemeWarningKind::Cycle },
        ];
        let lines = describe_theme_warnings(&warnings);
        assert_eq!(lines.len(), 2, "{:?}", lines);
        assert_eq!(
            lines[0],
            "style.toml: [panel.title] parent = \"acent\" does not name a selector — using defaults for it"
        );
        assert_eq!(
            lines[1],
            "style.toml: [transcript] parent = \"suggestion\" forms a parent cycle — using defaults for it"
        );
    }

    #[test]
    fn describe_theme_warnings_summarizes_beyond_the_threshold() {
        let warnings: Vec<ThemeWarning> = (0..6)
            .map(|i| ThemeWarning {
                selector: format!("sel{i}"),
                parent: "acent".into(),
                kind: ThemeWarningKind::UnknownParent,
            })
            .collect();
        let lines = describe_theme_warnings(&warnings);
        assert_eq!(lines.len(), 1, "collapses to one summary line: {:?}", lines);
        assert!(lines[0].contains("6 selectors"), "{}", lines[0]);
        assert!(lines[0].contains("sel0") && lines[0].contains("sel1") && lines[0].contains("sel2"));
        assert!(!lines[0].contains("sel3"), "only the first few are named: {}", lines[0]);
    }

    // ── SQ-0642: an fg/bg-only scheme must not go monochrome ─────────────────

    #[test]
    fn incomplete_palette_falls_back_per_role() {
        // A VALID Ghostty scheme needs only background+foreground; every unset
        // palette slot stays Reset. Roles derived from Reset slots used to send
        // line/accent/muted/alert — and every dependent selector — to Reset.
        let gs = GhosttyScheme::parse("background = 1d1f21\nforeground = c5c8c6\n").unwrap();
        let roles = Roles::from_scheme(&gs);
        assert_eq!(roles.text.fg, Some(Color::Rgb(0xc5, 0xc8, 0xc6)), "fg-derived roles keep the scheme");
        assert_eq!(roles.line.fg, Some(Color::Cyan), "empty cyan slot → terminal-default line role");
        assert_eq!(roles.accent.fg, Some(Color::Cyan));
        assert_eq!(roles.muted.fg, Some(Color::DarkGray));
        assert_eq!(roles.alert.fg, Some(Color::Yellow));

        let theme = resolve_theme(&gs, &ParsedStyle::default());
        assert_ne!(theme.get("map.connector").style.fg, Some(Color::Reset), "UI must not go monochrome");
        assert_eq!(theme.get("panel.border").style.fg, Some(Color::Cyan));
    }

    #[test]
    fn partially_filled_palette_uses_set_slots_and_falls_back_for_the_rest() {
        // palette[6] (cyan slot) is set; 3 and 8 are not.
        let gs = GhosttyScheme::parse(
            "background = 1d1f21\nforeground = c5c8c6\npalette = 6=#70c0ba\n",
        )
        .unwrap();
        let roles = Roles::from_scheme(&gs);
        assert_eq!(roles.line.fg, Some(Color::Rgb(0x70, 0xc0, 0xba)), "set slot is used");
        assert_eq!(roles.accent.fg, Some(Color::Rgb(0x70, 0xc0, 0xba)));
        assert_eq!(roles.muted.fg, Some(Color::DarkGray), "unset slot falls back per-role");
        assert_eq!(roles.alert.fg, Some(Color::Yellow));
    }

    #[test]
    fn user_parent_reroot_override_resolves() {
        // Re-rooting a selector onto a different parent makes it inherit that parent's
        // colour instead of its registry default (panel.title normally parents heading).
        let scheme = terminal_default_scheme();
        let parsed = super::super::toml_schema::parse("[panel]\ntitle = { parent = \"accent\" }\n").unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        assert_eq!(
            theme.get("panel.title").style.fg,
            theme.get("accent").style.fg,
            "panel.title re-rooted onto accent must inherit the accent colour",
        );
    }

    /// A re-root must move the COLOURS, and the case above cannot prove it.
    ///
    /// `panel.title`'s registry Delta is `Delta::EMPTY`, so its parent's colours
    /// reach it whether or not the mechanism is sound. The rows where a re-root
    /// can actually fail are the ones whose own Delta PINS an fg/bg — because
    /// `resolve_row` applies that Delta on top of the parent BEFORE any user
    /// decl, and a decl setting only `parent` never touches the colour channels.
    /// The pin therefore wins and the re-root is a silent no-op.
    ///
    /// Reported against `tooltip.background`, which pinned a warm-dark pair:
    /// `[tooltip] background = { parent = "dialog.list_selected" }` — the exact
    /// line a user wrote — changed nothing on screen. It resolves now because
    /// that row inherits instead of pinning.
    #[test]
    fn a_reroot_moves_the_colours_of_a_row_that_used_to_pin_them() {
        let scheme = terminal_default_scheme();
        let parsed =
            super::super::toml_schema::parse("[tooltip]\nbackground = { parent = \"alert\" }\n")
                .unwrap();
        let theme = resolve_theme(&scheme, &parsed);
        let tip = theme.get("tooltip.background").style;
        assert_eq!(
            tip.fg,
            theme.get("alert").style.fg,
            "a `parent` re-root of tooltip.background must carry the new parent's ink",
        );
        assert_ne!(
            tip.bg,
            Some(Color::Rgb(62, 54, 46)),
            "the retired warm-dark pin must not survive a re-root",
        );
    }

    // ── SQ-1169: a `parent` re-root must beat a row's pinned registry colours ─
    //
    // `resolve_row`'s step 1 used to apply the registry default `Delta` on top
    // of the resolved parent unconditionally — so a row whose own `Delta` pins
    // `fg`/`bg` restored that pin over whatever the user's `parent` override
    // supplied, silently. `dialog.list_selected` and `transcript_search_highlight`
    // pin BOTH channels (a total no-op); `status_header`, `dialog.shadow` and the
    // three `debug.disasm_*` tiers pin ONE (a half-op: the other channel moves,
    // which reads as fixed and is easy to mistake for it).

    /// Every registry row whose default `Delta` pins `fg` and/or `bg`, gathered
    /// from the registry itself rather than hand-listed, so a row added
    /// tomorrow with a pinned colour is covered the moment it lands.
    fn rows_pinning_a_colour() -> Vec<&'static RegRow> {
        REGISTRY
            .iter()
            .filter(|r| r.default_delta.fg.is_some() || r.default_delta.bg.is_some())
            .collect()
    }

    /// A re-root moves BOTH colour channels, for every row the registry pins
    /// one or both on (SQ-1169) — not just the seven the quest named by hand.
    /// `chrome` is the re-root target (`Roles::terminal_default().chrome` =
    /// White on Black): distinct from every pinned colour in the registry, so
    /// whichever channel stayed behind is caught.
    ///
    /// Falsifies against pre-fix `resolve_row`: every one of these rows keeps
    /// at least the channel its own `Delta` pins — e.g. `debug.disasm_executed`
    /// keeps `fg = Some(Color::Blue)` instead of following chrome's white, and
    /// `dialog.list_selected` keeps the whole `Black`/`Cyan` pair.
    #[test]
    fn a_reroot_moves_both_colour_channels_for_every_row_that_pins_one() {
        let roles = Roles::terminal_default();
        let chrome = roles.chrome;
        let rows = rows_pinning_a_colour();
        assert!(!rows.is_empty(), "sanity: the registry must still have pinned-colour rows to test");

        for row in rows {
            let decls = one(row.name, Delta { parent: Some("chrome".to_string()), ..Delta::EMPTY });
            let theme = resolve(&roles, &decls, &Decls::new(), &Decls::new());
            let got = theme.get(row.name).style;
            assert_eq!(
                got.fg, chrome.fg,
                "{:?}: re-rooting onto chrome must move fg to chrome's — a registry pin survived",
                row.name
            );
            assert_eq!(
                got.bg, chrome.bg,
                "{:?}: re-rooting onto chrome must move bg to chrome's — a registry pin survived",
                row.name
            );
        }
    }

    /// The seven rows SQ-1169 names explicitly, each with the exact pre-fix
    /// failure it reproduces — the generic sweep above proves the same thing
    /// mechanically; this spells out the specific report.
    #[test]
    fn seven_named_rows_move_both_channels_on_reroot() {
        let roles = Roles::terminal_default();
        let chrome = roles.chrome;
        let cases: &[(&str, &str)] = &[
            ("dialog.list_selected", "TOTAL no-op pre-fix: pinned Black+Cyan, both survived a re-root"),
            ("transcript_search_highlight", "TOTAL no-op pre-fix: pinned Black+Yellow, both survived"),
            ("status_header", "HALF pre-fix: bg pinned Black, stayed Black instead of following chrome"),
            ("dialog.shadow", "HALF pre-fix: bg pinned DarkGray, stayed DarkGray instead of following chrome"),
            ("debug.disasm_executed", "HALF pre-fix: fg pinned Blue, stayed Blue instead of following chrome"),
            ("debug.disasm_rd", "HALF pre-fix: fg pinned Yellow, stayed Yellow instead of following chrome"),
            ("debug.disasm_soft", "HALF pre-fix: fg pinned Red, stayed Red instead of following chrome"),
        ];
        for (name, why) in cases {
            let decls = one(name, Delta { parent: Some("chrome".to_string()), ..Delta::EMPTY });
            let theme = resolve(&roles, &decls, &Decls::new(), &Decls::new());
            let got = theme.get(name).style;
            assert_eq!(got.fg, chrome.fg, "{name}: {why}");
            assert_eq!(got.bg, chrome.bg, "{name}: {why}");
        }
    }

    /// No `parent` in the decl at all: the pinned/inherited pair is untouched.
    /// A user who only flips `bold` off must still see the registry's exact
    /// Black-on-Cyan — the re-root rule must not leak into the no-reroot case.
    #[test]
    fn no_reroot_still_pins_dialog_list_selected_colours() {
        let roles = Roles::terminal_default();
        let decls = one("dialog.list_selected", Delta { bold: Some(false), ..Delta::EMPTY });
        let theme = resolve(&roles, &decls, &Decls::new(), &Decls::new());
        let got = theme.get("dialog.list_selected").style;
        assert_eq!(got.fg, Some(Color::Black), "no parent override: the registry pin still applies");
        assert_eq!(got.bg, Some(Color::Cyan), "no parent override: the registry pin still applies");
        assert!(!got.add_modifier.contains(Modifier::BOLD), "bold = false must still switch the weight off");
    }

    /// A re-root moves colours, not the registry delta's own modifiers:
    /// `dialog.list_selected` is bold by default, and re-rooting it must keep
    /// that bold even though its pinned fg/bg get skipped.
    #[test]
    fn reroot_of_dialog_list_selected_keeps_bold() {
        let roles = Roles::terminal_default();
        let decls = one("dialog.list_selected", Delta { parent: Some("chrome".to_string()), ..Delta::EMPTY });
        let theme = resolve(&roles, &decls, &Decls::new(), &Decls::new());
        assert!(
            theme.get("dialog.list_selected").style.add_modifier.contains(Modifier::BOLD),
            "a re-root must not strip the registry delta's own bold"
        );
    }

    /// The tooltip's default IS the menu highlight, not a look of its own.
    ///
    /// `accent` cannot serve here however cyan it is: it is `fg(Cyan)` with no
    /// background, so deriving a borderless card from it repaints SQ-1139. The
    /// pair comes from `dialog.list_selected`, which every modal list already
    /// uses — so retuning that highlight moves the tooltip with it.
    #[test]
    fn the_tooltip_wears_the_shared_menu_highlight() {
        let theme = resolve(
            &Roles::terminal_default(),
            &Decls::new(),
            &Decls::new(),
            &Decls::new(),
        );
        let tip = theme.get("tooltip.background").style;
        let menu = theme.get("dialog.list_selected").style;
        assert_eq!((tip.fg, tip.bg), (menu.fg, menu.bg), "the tip wears the highlight's pair");
        assert!(tip.bg.is_some(), "a tip with no background cannot be a surface");
        assert_ne!(tip.fg, tip.bg, "and its ink must not be its own background");
    }

    /// …the COLOURS, not the weight (SQ-1171).
    ///
    /// The highlight's bold predates the registry — six hand-written
    /// `.add_modifier(BOLD)` literals that SQ-0643 consolidated verbatim. On one
    /// selected row it reads as "this one"; on a multi-line tooltip card it is a
    /// bold paragraph. The tip sheds it through the tri-state's erase arm.
    #[test]
    fn the_tooltip_takes_the_highlights_colours_but_not_its_weight() {
        let theme = resolve(
            &Roles::terminal_default(),
            &Decls::new(),
            &Decls::new(),
            &Decls::new(),
        );
        assert!(
            theme.get("dialog.list_selected").style.add_modifier.contains(Modifier::BOLD),
            "sanity: the menu highlight is still bold — otherwise this proves nothing",
        );
        assert!(
            !theme.get("tooltip.background").style.add_modifier.contains(Modifier::BOLD),
            "the tip must not inherit the highlight's weight",
        );
    }

    /// A modifier can be REMOVED, not merely added (SQ-1171).
    ///
    /// `apply_style` composed additively, so a child could only ever accumulate
    /// weight and `bold = false` in a user's style.toml was a silent no-op. The
    /// information was never missing — `RawDelta` has always parsed these as
    /// `Option<bool>` — it was discarded by an `unwrap_or(false)` on the way in.
    #[test]
    fn an_explicit_false_switches_a_modifier_off_where_absence_leaves_it_alone() {
        let scheme = terminal_default_scheme();
        // `heading` is the role that carries BOLD, so panel.title inherits it.
        let inherited = resolve_theme(&scheme, &super::super::toml_schema::parse("").unwrap());
        assert!(
            inherited.get("panel.title").style.add_modifier.contains(Modifier::BOLD),
            "sanity: panel.title inherits heading's bold by default",
        );

        let off = resolve_theme(
            &scheme,
            &super::super::toml_schema::parse("[panel]\ntitle = { bold = false }\n").unwrap(),
        );
        assert!(
            !off.get("panel.title").style.add_modifier.contains(Modifier::BOLD),
            "`bold = false` must switch the weight off, not be ignored",
        );

        // And an ABSENT key still inherits — the two must not collapse together,
        // which is the whole reason the channel is tri-state rather than a bool.
        let untouched = resolve_theme(
            &scheme,
            &super::super::toml_schema::parse("[panel]\ntitle = { italic = true }\n").unwrap(),
        );
        assert!(
            untouched.get("panel.title").style.add_modifier.contains(Modifier::BOLD),
            "saying nothing about bold must leave the parent's bold alone",
        );
    }

    // ── Task 0.3: layered decls + per-selector provenance ────────────────────

    #[test]
    fn per_game_layer_wins_and_is_stamped_pergame() {
        let roles = Roles::terminal_default();
        // Global and per-game both target `transcript`; per-game is the higher layer.
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let per_game = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &per_game);

        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Red)); // per-game value wins
        assert_eq!(r.provenance, Provenance::PerGame);
    }

    #[test]
    fn global_over_default_stamped_globaluser() {
        let roles = Roles::terminal_default();
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &Decls::new());

        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Green));
        assert_eq!(r.provenance, Provenance::GlobalUser);
    }

    #[test]
    fn garglk_between_global_and_pergame() {
        let roles = Roles::terminal_default();
        // All three layers target the same selector; the order per-game > garglk >
        // global > default must hold for both the value and the stamp.
        let global = one("transcript", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let garglk = one("transcript", Delta { fg: Some(Color::Blue), ..Delta::EMPTY });

        // garglk beats global when per-game is absent.
        let t1 = resolve(&roles, &global, &garglk, &Decls::new());
        assert_eq!(t1.get("transcript").style.fg, Some(Color::Blue));
        assert_eq!(t1.get("transcript").provenance, Provenance::Garglk);

        // per-game still beats garglk.
        let per_game = one("transcript", Delta { fg: Some(Color::Red), ..Delta::EMPTY });
        let t2 = resolve(&roles, &global, &garglk, &per_game);
        assert_eq!(t2.get("transcript").style.fg, Some(Color::Red));
        assert_eq!(t2.get("transcript").provenance, Provenance::PerGame);
    }

    #[test]
    fn unset_stays_default() {
        let roles = Roles::terminal_default();
        // A selector no layer touches keeps Provenance::Default.
        let global = one("status_bar", Delta { fg: Some(Color::Green), ..Delta::EMPTY });
        let theme = resolve(&roles, &global, &Decls::new(), &Decls::new());

        assert_eq!(theme.get("transcript").provenance, Provenance::Default);
        // And the touched selector is stamped, confirming Default isn't a blanket.
        assert_eq!(theme.get("status_bar").provenance, Provenance::GlobalUser);
    }

    // ── Task 1.2a: Roles::from_scheme + resolve_theme ─────────────────────────

    use super::super::toml_schema::ParsedStyle;
    use crate::colors::GhosttyScheme;

    /// A `GhosttyScheme` whose fg/bg/palette line up with
    /// `ColorScheme::terminal_default()` so `resolve_theme` should byte-match it.
    fn terminal_default_scheme() -> GhosttyScheme {
        let mut scheme = GhosttyScheme { foreground: Color::White, ..GhosttyScheme::default() };
        scheme.palette[3] = Color::Yellow; // alert slot (room_selected)
        scheme.palette[6] = Color::Cyan; // border/accent slot (focused_border/connector)
        scheme.palette[8] = Color::DarkGray; // muted slot (suggestion)
        scheme
    }

    #[test]
    fn no_scheme_falls_back_to_concrete_terminal_default_roles() {
        // Runtime regression guard: the startup `reload_style` builds the theme from
        // `resolve_base(None)` → `GhosttyScheme::default()`, whose fg/bg/palette are
        // all `Color::Reset`. Without the fallback, every role (hence every selector)
        // would resolve to `Reset` and the out-of-box look would go monochrome.
        let gs = crate::colors::GhosttyScheme::default();
        assert_eq!(gs.foreground, Color::Reset, "no-scheme base is all-Reset");
        assert_eq!(Roles::from_scheme(&gs), Roles::terminal_default());
        let theme = resolve_theme(&gs, &ParsedStyle::default());
        assert_eq!(theme.get("map.connector").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("panel.border").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("transcript").style.fg, Some(Color::White));
    }

    // ── SQ-0510: a probe-seeded scheme takes `from_scheme`'s real branch ──────

    #[test]
    fn a_probe_seeded_scheme_puts_the_terminal_page_under_the_chrome_roles() {
        // The reported bug: with no scheme configured the base is all-Reset, the
        // guard above early-returns, and `chrome` is a hard-coded white-on-BLACK —
        // so `upper_window` painted a black band across a terminal that is not
        // black. Once `colors::seed_scheme_from_terminal` has filled fg/bg in from
        // the OSC 10/11 probe, the real branch runs and chrome follows the terminal.
        let probe = crate::term_colors::TermDefaultColors {
            fg: Some(image::Rgba([0x58, 0x6e, 0x75, 255])),
            bg: Some(image::Rgba([0xfd, 0xf6, 0xe3, 255])), // Solarized Light page
        };
        let gs = crate::colors::seed_scheme_from_terminal(GhosttyScheme::default(), &probe);
        let roles = Roles::from_scheme(&gs);

        assert_ne!(roles.chrome.bg, Some(Color::Black), "the black guess is gone");
        assert_eq!(roles.chrome.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
        assert_eq!(roles.chrome.fg, Some(Color::Rgb(0x58, 0x6e, 0x75)));
        // The accents still come from the per-slot fallback — an all-Reset palette
        // must not drag the UI monochrome (SQ-0642's rule still holds here).
        assert_eq!(roles.line.fg, Some(Color::Cyan));
        assert_eq!(roles.accent.fg, Some(Color::Cyan));
        assert_eq!(roles.muted.fg, Some(Color::DarkGray));
        assert_eq!(roles.alert.fg, Some(Color::Yellow));
        // `text` keeps NO background, so the transcript still shows the terminal
        // page through — chrome now matches it instead of contradicting it.
        assert_eq!(roles.text.bg, None);

        // …and every chrome-derived selector inherits the probed page.
        let theme = resolve_theme(&gs, &ParsedStyle::default());
        for sel in ["upper_window", "status_bar", "story_info", "dialog.background", "glk.grid.normal", "glk.grid.background"] {
            assert_eq!(
                theme.get(sel).style.bg,
                Some(Color::Rgb(0xfd, 0xf6, 0xe3)),
                "{sel} must follow the terminal page, not a hard-coded black"
            );
        }
    }

    #[test]
    fn an_unanswered_probe_leaves_the_terminal_default_roles_alone() {
        // The other half: a terminal that answers neither query keeps today's
        // behaviour byte-for-byte — the seeding is a no-op and `from_scheme` still
        // early-returns to the concrete terminal-default palette.
        let gs = crate::colors::seed_scheme_from_terminal(
            GhosttyScheme::default(),
            &crate::term_colors::TermDefaultColors::default(),
        );
        assert_eq!(Roles::from_scheme(&gs), Roles::terminal_default());
        assert_eq!(Roles::from_scheme(&gs).chrome.bg, Some(Color::Black));
    }

    #[test]
    fn resolve_theme_reproduces_terminal_defaults() {
        let scheme = terminal_default_scheme();
        let theme = resolve_theme(&scheme, &ParsedStyle::default());

        assert_eq!(theme.get("transcript").style.fg, Some(Color::White));
        assert_eq!(theme.get("map.connector").style.fg, Some(Color::Cyan));
        assert_eq!(theme.get("map.connector_distorted").style.fg, Some(Color::Magenta));
        assert_eq!(theme.get("map.shared_path").style.fg, Some(Color::LightCyan));
        assert_eq!(theme.get("panel.border").style.fg, Some(Color::Cyan));

        let border_active = theme.get("panel.border:active").style;
        assert_eq!(border_active.fg, Some(Color::Cyan));
        assert!(border_active.add_modifier.contains(Modifier::BOLD));

        assert!(theme.get("status_bar").style.add_modifier.contains(Modifier::REVERSED));
        assert!(theme.get("help_bar").style.add_modifier.contains(Modifier::REVERSED));
        // SQ-1212: a Glk grid's ground is reversed chrome, the same spelling.
        assert!(theme.get("glk.grid.background").style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(theme.get("suggestion").style.fg, Some(Color::DarkGray));
        assert!(theme.get("glk.buffer.header").style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn parsed_roles_and_decls_lower() {
        let scheme = terminal_default_scheme();

        // (a) a [roles] override on `accent` flows to a role-derived selector.
        let mut with_role = ParsedStyle::default();
        with_role.roles.insert(
            "accent".to_string(),
            super::super::toml_schema::RawDelta {
                fg: Some("red".to_string()),
                ..super::super::toml_schema::RawDelta::default()
            },
        );
        let theme = resolve_theme(&scheme, &with_role);
        assert_eq!(theme.get("map.room_current").style.fg, Some(Color::Red));

        // (b) a decl override on `transcript` wins over its default (text role).
        let mut with_decl = ParsedStyle::default();
        with_decl.decls.insert(
            "transcript".to_string(),
            super::super::toml_schema::RawDelta {
                fg: Some("green".to_string()),
                ..super::super::toml_schema::RawDelta::default()
            },
        );
        let theme = resolve_theme(&scheme, &with_decl);
        assert_eq!(theme.get("transcript").style.fg, Some(Color::Green));
    }

    // ── Task 3.1: resolve_theme_layered (global + per-game, provenance) ──────

    #[test]
    fn layered_per_game_beats_global_with_provenance() {
        use super::super::toml_schema::RawDelta;
        let scheme = terminal_default_scheme();

        let mut global = ParsedStyle::default();
        global.decls.insert(
            "transcript".to_string(),
            RawDelta { fg: Some("green".to_string()), ..RawDelta::default() },
        );

        // per_game overrides transcript to red.
        let mut per_game = ParsedStyle::default();
        per_game.decls.insert(
            "transcript".to_string(),
            RawDelta { fg: Some("red".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &per_game);
        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Red));
        assert_eq!(r.provenance, Provenance::PerGame);

        // With per_game default (no override), global wins.
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &ParsedStyle::default());
        let r = theme.get("transcript");
        assert_eq!(r.style.fg, Some(Color::Green));
        assert_eq!(r.provenance, Provenance::GlobalUser);
    }

    #[test]
    fn layered_roles_override_applies() {
        use super::super::toml_schema::RawDelta;
        let scheme = terminal_default_scheme();

        let mut global = ParsedStyle::default();
        global.roles.insert(
            "accent".to_string(),
            RawDelta { fg: Some("green".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &ParsedStyle::default());
        assert_eq!(theme.get("accent").style.fg, Some(Color::Green));

        // A per-game [roles] override wins over the global one.
        let mut per_game = ParsedStyle::default();
        per_game.roles.insert(
            "accent".to_string(),
            RawDelta { fg: Some("red".to_string()), ..RawDelta::default() },
        );
        let theme = resolve_theme_layered(&scheme, &global, &Decls::new(), &per_game);
        assert_eq!(theme.get("accent").style.fg, Some(Color::Red));
    }
}
