//! Bottom hint-bar view builder: turns the live keymap + hotkey layout into the
//! context-appropriate " key: label | … " string drawn along the bottom row.
//! Extracted verbatim from `main.rs` (SQ-0306) as a pure move — no behavior
//! change. A pure view builder, so it lives in `render/` alongside its siblings.

use crate::debug_panel::Section;
use crate::keymap::{Context, HotkeyLayout, KeyMap};

/// Priority-ordered command-string lists for the bottom hint bar.
/// Commands are included only when directly available in the current context.
/// `tidy-map` is intentionally excluded from all lists.
/// Story-pane hints while the debug inspector is open, so Tab has somewhere to
/// go.
pub const GAME_HINTS: &[&str] = &[
    "toggle-focus",
    "save-state",
    "restore-state",
];

/// The ordinary story-pane hints. `toggle-focus` is deliberately absent: with
/// the inspector closed the map is not a focus stop (SQ-0599), so Tab does
/// nothing and advertising it would promise a mode that no longer exists.
pub const GAME_HINTS_NO_INSPECTOR: &[&str] = &[
    "save-state",
    "restore-state",
];

pub const ANIM_HINTS: &[&str] = &[
    "anim-step forward",
    "anim-play",
    "anim-exit",
    "pan-map -1 0",
    "zoom-map in",
];

/// Debug-inspector hint bar, the part that holds in every section: window and
/// tab navigation, scrolling, paging, and the way out. Unlike the other hint
/// lists above, these bindings are internal to the debug panel (handled by
/// `DebugPanelState::handle_key`), not registered commands in the global
/// keymap — so they're rendered via [`literal_hint_bar`] instead of
/// [`hint_bar`]'s keymap-resolution path.
///
/// These come **last** in the assembled bar (see [`debug_hints`]) precisely
/// because [`literal_hint_bar`] truncates from the right: Tab, the arrows and
/// Esc are conventions a user will try unprompted, so losing them off the end
/// of a narrow pane costs far less than losing a key that exists in one
/// section only.
pub const DEBUG_HINTS_UNIVERSAL: &[(&str, &str)] = &[
    ("Tab", "window"),
    ("\u{2190}\u{2192}", "section"),
    ("\u{2191}\u{2193}", "scroll"),
    ("PgUp/PgDn", "page"),
    // Advertised only once they meant one thing (SQ-0984): they jumped to the
    // extremes in the list sections and moved a single instruction or hex row in
    // Disassembly and Memory, and a key that needs two descriptions cannot have
    // an entry here.
    ("Home/End", "ends"),
    ("Esc", "back"),
];

/// The keys that exist only in `section`. Empty for every section whose only
/// bindings are the universal ones — those show [`DEBUG_HINTS_UNIVERSAL`]
/// alone rather than advertising a key that does nothing where the user is
/// standing.
///
/// `g` is listed under Disassembly rather than universally because that is where
/// it works. It used to be accepted from any tab while only ever re-anchoring the
/// disassembly — invisible from anywhere else, and the handler is now gated to
/// match this list rather than the other way round (SQ-0984).
fn debug_section_hints(
    section: Section,
    disasm_mode: &'static str,
) -> Vec<(&'static str, &'static str)> {
    match section {
        Section::Disasm => vec![("g", "PC"), ("r", disasm_mode)],
        // `:` (or `/`) opens the address box, which also takes a variable
        // token (`sp`, `g44`, `local10`) — never advertised before SQ-0980.
        Section::Memory => vec![("hl", "pan"), (":", "address")],
        _ => Vec::new(),
    }
}

/// The debug hint bar for the focused window's active `section`: that
/// section's own keys first, then [`DEBUG_HINTS_UNIVERSAL`].
///
/// Section-specific first is the whole point (SQ-0980). The old fixed list
/// advertised `hl: pan` in tabs that cannot pan and `r: raw` in tabs that have
/// no render mode to cycle, while [`literal_hint_bar`]'s right-hand truncation
/// cut those same entries off first at a debug pane's real width — so the bar
/// hid the local keys and promised the absent ones in one stroke.
///
/// `disasm_mode` is [`crate::debug_panel::DebugPanelState::disasm_mode_label`],
/// so the `r:` entry names the mode currently showing.
pub fn debug_hints(
    section: Section,
    disasm_mode: &'static str,
) -> Vec<(&'static str, &'static str)> {
    let mut hints = debug_section_hints(section, disasm_mode);
    hints.extend_from_slice(DEBUG_HINTS_UNIVERSAL);
    hints
}

/// Build a hint bar string from a fixed literal `(key, label)` list. Mirrors
/// [`hint_bar`]'s join/truncate behavior, without the keymap-resolution gates
/// (there is no keymap entry to resolve — the bindings are hard-coded).
pub fn literal_hint_bar(hints: &[(&str, &str)], width: usize) -> String {
    let joined = hints.iter().map(|(k, l)| format!("{k}: {l}")).collect::<Vec<_>>().join(" | ");
    if width == 0 {
        return String::new();
    }
    let char_count = joined.chars().count();
    if char_count <= width {
        joined
    } else {
        let truncate_at = width.saturating_sub(1);
        let byte_pos = joined
            .char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(joined.len());
        format!("{}…", &joined[..byte_pos])
    }
}

/// Short hint-bar label for a command-string: "zoom-map in" -> "zoom map in".
fn hint_label(cmd_str: &str) -> String {
    cmd_str.replace('-', " ")
}

/// Build the hint bar string for the given context from the live keymap and layout.
///
/// For each command-string in `priority`, an entry is included only if all three hold:
/// 1. `layout.is_direct_name(cmd)` — the command is directly available, not dialog-only.
/// 2. `keymap.primary_key(name)` returns a KeySpec `k`.
/// 3. `keymap.lookup(&k, ctx) == Some(cmd)` — pressing `k` in `ctx` resolves to `cmd`.
///
/// Each surviving entry renders as "{k.label()}: {label}"; entries join with " | ".
/// If the joined string exceeds `width` characters, it is truncated and "…" appended.
pub fn hint_bar(
    keymap: &KeyMap,
    layout: &HotkeyLayout,
    ctx: Context,
    priority: &[&str],
    width: usize,
) -> String {
    let entries: Vec<String> = priority
        .iter()
        .filter_map(|&cmd| {
            // Gate 1: command must be directly available (not dialog-only).
            if !layout.is_direct_name(cmd) {
                return None;
            }
            // Gate 2: command must have a primary key binding.
            let name = cmd.split_whitespace().next().unwrap_or("");
            let k = keymap.primary_key(name)?;
            // Gate 3: pressing that key in this context must resolve back to this command.
            if keymap.lookup(&k, ctx) != Some(cmd) {
                return None;
            }
            let label = hint_label(cmd);
            Some(format!("{}: {}", k.label(), label))
        })
        .collect();

    let joined = entries.join(" | ");

    // Truncate to width (char-count aware), appending "…" if needed.
    if width == 0 {
        return String::new();
    }
    let char_count = joined.chars().count();
    if char_count <= width {
        joined
    } else {
        // Find the byte offset after (width - 1) chars to leave room for "…".
        let truncate_at = width.saturating_sub(1);
        let byte_pos = joined
            .char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(joined.len());
        format!("{}…", &joined[..byte_pos])
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::{debug_hints, hint_bar, literal_hint_bar, ANIM_HINTS, GAME_HINTS};
    use crate::debug_panel::Section;
    use crate::keymap::{Context, HotkeyLayout, KeyMap};

    /// The universal tail, spelled out once so the per-section cases below
    /// assert on the whole line rather than on fragments of it.
    const TAIL: &str = "Tab: window | \u{2190}\u{2192}: section | \u{2191}\u{2193}: scroll | PgUp/PgDn: page | Home/End: ends | Esc: back";

    #[test]
    fn literal_hint_bar_joins_debug_hints() {
        // Was a single fixed list for every tab; now the exact line per section
        // (SQ-0980). Disassembly leads with its own two keys, and the `r:`
        // entry names the live render mode.
        let line = literal_hint_bar(&debug_hints(Section::Disasm, "full"), 200);
        assert_eq!(line, format!("g: PC | r: full | {TAIL}"));
        let line = literal_hint_bar(&debug_hints(Section::Memory, "full"), 200);
        assert_eq!(line, format!("hl: pan | :: address | {TAIL}"));
        // A section with no keys of its own shows the universal set alone.
        let line = literal_hint_bar(&debug_hints(Section::Globals, "full"), 200);
        assert_eq!(line, TAIL);
    }

    /// SQ-0980: the bar advertised `hl: pan` and `r: raw` in every tab, so it
    /// promised keys that do nothing where the user was standing.
    #[test]
    fn a_section_is_never_offered_another_sections_keys() {
        for section in [
            Section::Globals,
            Section::Locals,
            Section::Objects,
            Section::Dict,
            Section::CallStack,
            Section::EvalStack,
        ] {
            let line = literal_hint_bar(&debug_hints(section, "full"), 200);
            assert!(!line.contains("pan"), "{section:?} cannot pan: {line}");
            assert!(!line.contains("address"), "{section:?} has no address box: {line}");
            assert!(!line.contains("g: PC"), "{section:?} shows no disassembly: {line}");
            assert!(!line.contains("r: "), "{section:?} has no render mode: {line}");
        }
        let disasm = literal_hint_bar(&debug_hints(Section::Disasm, "raw"), 200);
        assert!(!disasm.contains("pan"), "the disassembly does not pan: {disasm}");
        assert!(!disasm.contains(": address"), "no address box here: {disasm}");
        let memory = literal_hint_bar(&debug_hints(Section::Memory, "raw"), 200);
        assert!(!memory.contains("g: PC"), "the hex dump has no PC: {memory}");
        assert!(!memory.contains("r: "), "and no render mode: {memory}");
    }

    /// The ordering exists for this: `literal_hint_bar` truncates from the
    /// RIGHT, so whatever a narrow debug pane can still show must be the keys
    /// that only work here. A user who could not find the pan they had just
    /// been given is what started SQ-0980.
    #[test]
    fn a_narrow_pane_keeps_the_local_keys_and_drops_the_conventional_ones() {
        // 24 columns is narrower than a real debug pane, and the point still
        // has to hold there.
        let line = literal_hint_bar(&debug_hints(Section::Memory, "full"), 24);
        assert!(line.chars().count() <= 24, "{line}");
        assert!(line.starts_with("hl: pan | :: address"), "the pan survives: {line}");
        assert!(line.ends_with('…'), "and the universal tail is what got cut: {line}");
        let line = literal_hint_bar(&debug_hints(Section::Disasm, "basic"), 24);
        assert!(line.starts_with("g: PC | r: basic"), "{line}");
    }

    /// SQ-0984: `Home`/`End` work in every section, so every section says so.
    ///
    /// They were left off the bar because they did not mean one thing — jump to
    /// the extremes in the list sections, one instruction or one hex row in
    /// Disassembly and Memory. Now that they do, the omission would be the same
    /// defect as SQ-0980's the other way round: a key that works and is never
    /// mentioned.
    #[test]
    fn every_section_advertises_the_keys_that_hold_everywhere() {
        for section in [
            Section::Disasm,
            Section::Globals,
            Section::Locals,
            Section::Objects,
            Section::Dict,
            Section::CallStack,
            Section::EvalStack,
            Section::Memory,
        ] {
            let line = literal_hint_bar(&debug_hints(section, "full"), 200);
            assert!(line.contains("Home/End: ends"), "{section:?} jumps to the ends: {line}");
            assert!(line.contains("PgUp/PgDn: page"), "{section:?} pages: {line}");
        }
    }

    #[test]
    fn literal_hint_bar_truncates_at_width() {
        let line = literal_hint_bar(&debug_hints(Section::Memory, "full"), 10);
        assert!(line.chars().count() <= 10);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn hint_line_anim_contains_zoom_with_plus_key() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        // With default keymap: zoom-map in primary key is '+'; short label is "zoom map in".
        assert!(line.contains("+: zoom"), "expected '+: zoom' in '{line}'");
    }

    #[test]
    fn hint_bar_excludes_dialog_only_commands() {
        // Regression (#11): gallery/inspector/layout moved to the leader dialog
        // after the leader-key change; the hint bar must NOT advertise their dead
        // direct keys. The is_direct filter excludes them (they are dialog-only).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        assert!(!line.contains("gallery"), "must not advertise gallery (dialog-only): {line}");
        assert!(!line.contains("inspector"), "must not advertise inspector (dialog-only): {line}");
        assert!(!line.contains("hide the map"), "must not advertise toggle-map (dialog-only): {line}");
        // The working direct keys ARE present.
        assert!(line.contains("+: zoom"), "zoom present: {line}");
    }

    /// SQ-0599: the story hint bar only offers Tab when the inspector gives it
    /// somewhere to go. Advertising a focus toggle that does nothing is exactly
    /// the invisible-mode confusion this change removed.
    #[test]
    fn the_story_hint_bar_offers_tab_only_with_the_inspector_open() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let plain = hint_bar(&km, &layout, Context::Global, super::GAME_HINTS_NO_INSPECTOR, 200);
        assert!(!plain.contains("toggle focus"), "no focus toggle without the inspector: {plain}");
        assert!(plain.contains("save state"), "the real global keys are still shown: {plain}");
        let with_inspector = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        assert!(with_inspector.contains("Tab: toggle focus"), "{with_inspector}");
    }

    #[test]
    fn hint_line_game_contains_save_state() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        // Ctrl+S → save-state; short label is "save state".
        assert!(line.contains("Ctrl+S: save state"), "expected 'Ctrl+S: save state' in '{line}'");
        // toggle-map (formerly cycle-layout) was trimmed out of the always-active
        // set (SQ-0202); it's leader-only now and must not appear in the Game hint bar.
        assert!(!line.contains("hide the map"), "Game hint bar must not advertise toggle-map: '{line}'");
    }

    #[test]
    fn leader_hint_advertises_ctrl_p_menu() {
        // The bottom-bar default branch prepends "{prefix.label()}: menu" ahead
        // of the hint_bar output (SQ-0202). Pin the exact construction here since
        // the help-row assembly itself lives inline in the render loop. Prefix
        // moved from Ctrl+K to Ctrl+P (SQ-0447), freeing Ctrl+K for the story
        // prompt's readline delete-to-end shortcut.
        let layout = HotkeyLayout::default();
        let leader_hint = format!("{}: menu", layout.prefix.label());
        assert_eq!(leader_hint, "Ctrl+P: menu");
    }

    #[test]
    fn hint_bar_never_contains_tidy() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let anim_line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        let game_line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        assert!(
            !anim_line.to_lowercase().contains("tidy") && !anim_line.to_lowercase().contains("retidy"),
            "anim hint bar must not contain tidy/retidy; got: '{anim_line}'"
        );
        assert!(
            !game_line.to_lowercase().contains("tidy") && !game_line.to_lowercase().contains("retidy"),
            "game hint bar must not contain tidy/retidy; got: '{game_line}'"
        );
    }

    #[test]
    fn hint_bar_no_dead_keys_all_entries_resolve_back() {
        // Every entry shown must pass the round-trip check: lookup(primary_key(cmd), ctx) == Some(cmd).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        for (ctx, hints) in [
            (Context::Global, GAME_HINTS),
            (Context::Global, super::GAME_HINTS_NO_INSPECTOR),
            (Context::Anim, ANIM_HINTS),
        ] {
            for &cmd in hints {
                if !layout.is_direct_name(cmd) {
                    continue;
                }
                let name = cmd.split_whitespace().next().unwrap_or("");
                if let Some(k) = km.primary_key(name) {
                    let resolved = km.lookup(&k, ctx);
                    if resolved == Some(cmd) {
                        // This entry would be shown — verify label format.
                        let entry = format!("{}: {}", k.label(), super::hint_label(cmd));
                        let bar = hint_bar(&km, &layout, ctx, hints, 200);
                        assert!(
                            bar.contains(&entry),
                            "bar for {ctx:?} should contain '{entry}'; got: '{bar}'"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hint_bar_drops_non_direct_command() {
        use std::collections::HashSet;
        // Build a layout where zoom-map in is NOT direct (dialog-only), but toggle-focus IS.
        let mut direct: HashSet<String> = HashSet::new();
        direct.insert("toggle-focus".into());
        direct.insert("anim-play".into());
        // zoom-map in intentionally NOT in direct set.
        let layout = HotkeyLayout {
            prefix: "ctrl+k".parse().unwrap(),
            direct,
            groups: vec![],
        };
        let km = KeyMap::default();
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        assert!(
            !bar.contains("zoom"),
            "zoom-map in should be absent when not direct; got: '{bar}'"
        );
        // anim-play IS direct, so it should appear.
        assert!(
            bar.contains("anim play"),
            "anim-play should still appear when direct; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_truncates_at_width() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very narrow width (10 chars) — the full bar is much longer.
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 10);
        let char_count = bar.chars().count();
        assert!(
            char_count <= 10,
            "bar must not exceed width=10; got {char_count} chars: '{bar}'"
        );
        assert!(
            bar.ends_with('…'),
            "truncated bar must end with ellipsis; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_no_truncation_when_wide_enough() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very generous width — no truncation expected.
        let bar = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 1000);
        assert!(
            !bar.ends_with('…'),
            "bar should not be truncated at width=1000; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_shows_short_registry_labels() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Anim, ANIM_HINTS, 200);
        // zoom-map in is direct and bound to '+' in Anim; its short label is "zoom map in".
        assert!(line.contains("zoom map in"), "hint bar should show the short label 'zoom map in', got: {line}");
        // The full description sentence must NOT appear.
        assert!(!line.contains("zoom the map in/out"), "hint bar must not show the long description");
    }
}
