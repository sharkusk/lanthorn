//! Styling-role redesign (SQ-0309): the declarative selector registry and the
//! resolver/theme model built on top of it.
//!
//! [`registry`] is the single source of truth — one row per themeable selector
//! (name, section, kind, parent, default delta). Later tasks add the resolver
//! and TOML schema that consume it.

pub mod registry;
pub mod resolve;
pub mod template;
pub mod toml_schema;

// ── describe_theme ──────────────────────────────────────────────────────────

/// Describe a resolved [`resolve::Theme`] as printable lines for the `/colors`
/// dump: a header per registry [`registry::Section`] (style `None`), then one
/// line per selector `  <name>: fg=<fg> bg=<bg><attrs>` carrying that
/// selector's resolved [`ratatui::style::Style`]. Mirrors the shape of the
/// pre-SQ-0309 `style::describe_scheme` grouped output. `Section::Statusbar`
/// is skipped (dynamic `[[statusbar.segment]]` rows, no fixed registry rows).
pub fn describe_theme(theme: &resolve::Theme) -> Vec<(String, Option<ratatui::style::Style>)> {
    use ratatui::style::Modifier;

    const SECTIONS: &[(registry::Section, &str)] = &[
        (registry::Section::Roles, "roles"),
        (registry::Section::Elements, "elements"),
        (registry::Section::Panel, "panel"),
        (registry::Section::GlkBuffer, "glk.buffer"),
        (registry::Section::GlkGrid, "glk.grid"),
        (registry::Section::Map, "map"),
        (registry::Section::Debug, "debug"),
        // SQ-0642: the surface sections were missing, so /colors never listed
        // the dialog.* / tooltip.* selectors. Only Statusbar is intentionally
        // skipped (dynamic segment rows, no fixed registry rows).
        (registry::Section::Dialog, "dialog"),
        (registry::Section::Tooltip, "tooltip"),
    ];

    let mut out: Vec<(String, Option<ratatui::style::Style>)> = Vec::new();
    for (section, title) in SECTIONS {
        out.push((format!("── {title} ──"), None));
        for row in registry::REGISTRY.iter().filter(|r| r.section == *section) {
            let st = theme.get(row.name).style;
            let fg = st.fg.map(crate::style::color_to_str).unwrap_or_else(|| "default".to_string());
            let bg = st.bg.map(crate::style::color_to_str).unwrap_or_else(|| "default".to_string());
            let mut attrs: Vec<&str> = Vec::new();
            if st.add_modifier.contains(Modifier::BOLD) { attrs.push("bold"); }
            if st.add_modifier.contains(Modifier::ITALIC) { attrs.push("italic"); }
            if st.add_modifier.contains(Modifier::UNDERLINED) { attrs.push("underline"); }
            if st.add_modifier.contains(Modifier::DIM) { attrs.push("dim"); }
            if st.add_modifier.contains(Modifier::REVERSED) { attrs.push("reversed"); }
            let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(",")) };
            out.push((format!("  {}: fg={fg} bg={bg}{attr_str}", row.name), Some(st)));
        }
    }
    out
}

#[cfg(all(test, feature = "t-theme"))]
mod tests {
    use super::*;

    #[test]
    fn describe_theme_lists_every_section_except_statusbar() {
        // SQ-0642: Dialog and Tooltip were missing from describe_theme's SECTIONS,
        // so /colors never listed dialog.* / tooltip.* selectors. Only Statusbar
        // (dynamic rows, no fixed registry entries) is intentionally skipped.
        let theme = resolve::resolve(
            &resolve::Roles::terminal_default(),
            &resolve::Decls::new(),
            &resolve::Decls::new(),
            &resolve::Decls::new(),
        );
        let lines = describe_theme(&theme);
        let has = |needle: &str| lines.iter().any(|(l, _)| l.contains(needle));
        assert!(has("dialog.shadow"), "dialog.* selectors must be listed");
        assert!(has("dialog.border"));
        assert!(has("tooltip.border"), "tooltip.* selectors must be listed");
        assert!(has("tooltip.background"));
        assert!(has("── dialog ──") && has("── tooltip ──"), "section headers present");

        // Completeness: every non-Statusbar registry row appears exactly once.
        for row in registry::REGISTRY.iter().filter(|r| r.section != registry::Section::Statusbar) {
            assert!(
                lines.iter().any(|(l, _)| l.trim_start().starts_with(&format!("{}:", row.name))),
                "registry row {:?} missing from describe_theme output",
                row.name
            );
        }
    }
}
