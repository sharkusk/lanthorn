//! Generates the four tables under `docs/reference/` straight from the app's
//! own registries — the slash-command list, the default keymap, the config
//! template, and the style/theme registry — so the reference can never drift
//! from what the code actually does.
//!
//! Nothing here is read at runtime. `crates/app/tests/suites/docs_reference.rs`
//! calls each `render_*` function and diffs it against the committed file;
//! `LANTHORN_REGEN_DOCS=1` makes that test WRITE the file instead of asserting.
//! See that suite for the regeneration instructions.

/// Escape a plain-text table cell so a literal `|` in it cannot be read as a
/// column delimiter. Not used on text already wrapped in a code span — GFM
/// table parsing is code-span-aware, so a `|` inside `` `…` `` needs no escape.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|")
}

// ── commands.md ─────────────────────────────────────────────────────────────

/// Render `docs/reference/commands.md` from `slash::COMMANDS`.
pub fn render_commands() -> String {
    let mut out = String::new();
    out.push_str(
        "<!-- generated from crates/app/src/slash.rs (slash::COMMANDS) by docs_reference; do not edit by hand -->\n",
    );
    out.push_str("# Command reference\n\n");
    out.push_str(
        "Every slash command, grouped the way `/help` groups them. Type any of these \
         after the command prefix (`/` by default); a key bound to one is in \
         [keys.md](keys.md).\n\n",
    );
    out.push_str("| Category | Command | Description |\n");
    out.push_str("|---|---|---|\n");
    for cat in crate::slash::Category::ORDER {
        for c in crate::slash::COMMANDS.iter().filter(|c| c.category == cat) {
            out.push_str(&format!(
                "| {} | `{}` | {} |\n",
                cat.title(),
                c.usage,
                md_cell(c.description)
            ));
        }
    }
    out
}

// ── keys.md ─────────────────────────────────────────────────────────────────

/// Render `docs/reference/keys.md` from `keymap::KeyMap::default()`.
pub fn render_keys() -> String {
    let mut out = String::new();
    out.push_str(
        "<!-- generated from crates/app/src/keymap.rs (keymap::KeyMap::default) by docs_reference; do not edit by hand -->\n",
    );
    out.push_str("# Key reference\n\n");
    out.push_str(
        "The built-in key bindings, one row per binding — several keys may reach the same \
         command. Rebind any of these under `[keymap.*]` in `config.toml`; see \
         [config.md](config.md).\n\n",
    );
    out.push_str("| Context | Key | Command | Description |\n");
    out.push_str("|---|---|---|---|\n");
    let km = crate::keymap::KeyMap::default();
    for (spec, cmd, ctx) in &km.bindings {
        let name = cmd.split_whitespace().next().unwrap_or("");
        let desc = crate::slash::find_command(name).map(|c| c.description).unwrap_or("");
        out.push_str(&format!(
            "| {:?} | `{}` | `{}` | {} |\n",
            ctx,
            spec.label(),
            cmd,
            md_cell(desc)
        ));
    }
    out
}

// ── config.md ───────────────────────────────────────────────────────────────

/// Render `docs/reference/config.md` from `config_template::GROUPS`.
pub fn render_config() -> String {
    let mut out = String::new();
    out.push_str(
        "<!-- generated from crates/app/src/config_template.rs (config_template::GROUPS) by docs_reference; do not edit by hand -->\n",
    );
    out.push_str("# Config reference\n\n");
    out.push_str(
        "Every setting `~/.lanthorn/config.toml` accepts, grouped the way the seeded template \
         groups them. \"example\" means the default cannot be written down (unset/computed) and \
         the value shown only illustrates the shape; \"live default\" means the setting ships \
         uncommented because it is content rather than documentation.\n\n",
    );
    for g in crate::config_template::GROUPS {
        out.push_str(&format!("## {}\n\n", g.banner));
        out.push_str("| Key | Default | Note | Description |\n");
        out.push_str("|---|---|---|---|\n");
        for row in g.rows {
            let key = match g.table {
                Some(t) => format!("{t}.{}", row.key),
                None => row.key.to_string(),
            };
            let note = match row.line {
                crate::config_template::Line::Default => "",
                crate::config_template::Line::Example => "example",
                crate::config_template::Line::Live => "live default",
            };
            let desc = row.doc.join(" ");
            out.push_str(&format!(
                "| `{key}` | `{}` | {note} | {} |\n",
                row.value,
                md_cell(&desc)
            ));
        }
        out.push('\n');
    }
    out
}

// ── style.md ────────────────────────────────────────────────────────────────

/// A short, boring summary of a [`crate::theme::registry::Delta`] — every
/// channel it sets, none it does not. Used only for `style.md`'s "Default"
/// column, so a themer can see what a selector adds on top of its parent
/// without reading `theme/registry.rs`.
fn summarize_delta(d: &crate::theme::registry::Delta) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = d.fg {
        parts.push(format!("fg={c:?}"));
    }
    if let Some(c) = d.bg {
        parts.push(format!("bg={c:?}"));
    }
    for (name, v) in [
        ("bold", d.bold),
        ("italic", d.italic),
        ("underline", d.underline),
        ("reversed", d.reversed),
        ("dim", d.dim),
    ] {
        match v {
            Some(true) => parts.push(name.to_string()),
            Some(false) => parts.push(format!("!{name}")),
            None => {}
        }
    }
    if let Some(g) = &d.glyph {
        parts.push(format!("glyph={g:?}"));
    }
    if let Some(b) = d.border {
        parts.push(format!("border={b:?}"));
    }
    if let Some(b) = d.border_top {
        parts.push(format!("border_top={b:?}"));
    }
    if let Some(b) = d.border_bottom {
        parts.push(format!("border_bottom={b:?}"));
    }
    if let Some(b) = d.border_left {
        parts.push(format!("border_left={b:?}"));
    }
    if let Some(b) = d.border_right {
        parts.push(format!("border_right={b:?}"));
    }
    if let Some(h) = d.header {
        parts.push(format!("header={h}"));
    }
    if let Some(s) = d.shadow {
        parts.push(format!("shadow={s}"));
    }
    if let Some(p) = &d.parent {
        parts.push(format!("parent->{p}"));
    }
    parts.join(" ")
}

/// Render `docs/reference/style.md` from `theme::registry::REGISTRY`.
pub fn render_style() -> String {
    let mut out = String::new();
    out.push_str(
        "<!-- generated from crates/app/src/theme/registry.rs (theme::registry::REGISTRY) by docs_reference; do not edit by hand -->\n",
    );
    out.push_str("# Style reference\n\n");
    out.push_str(
        "Every themeable `style.toml` selector: which role or selector it derives from, and \
         what its built-in default changes on top of that parent. An empty Default cell \
         inherits its parent exactly.\n\n",
    );
    out.push_str("| Selector | Section | Kind | Parent | Default | Description |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for row in crate::theme::registry::REGISTRY.iter() {
        let parent = match row.parent {
            Some(p) => format!("`{p}`"),
            None => String::new(),
        };
        let default = summarize_delta(&row.default_delta);
        let default = if default.is_empty() { String::new() } else { format!("`{default}`") };
        out.push_str(&format!(
            "| `{}` | {:?} | {:?} | {parent} | {default} | |\n",
            row.name, row.section, row.kind
        ));
    }
    out
}
