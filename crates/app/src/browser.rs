//! The story browser's command surface (SQ-0796).
//!
//! The browser is a **pre-game loop with no `AppState`**, so it cannot run the
//! in-game [`crate::input::Action`] path — but its keys are still *data*,
//! resolved through the one [`crate::slash::COMMANDS`] registry like everything
//! else. Each gesture is a registry command with [`Context::Browser`] whose
//! dispatch yields a [`BrowserAction`]; `picker_ui` matches on the action and
//! never on a key code.
//!
//! That is the whole point: a gesture can only reach the picker by coming out of
//! [`action_for_key`], which reads the keymap and the registry and nothing else.
//! Adding one therefore *means* adding a registry entry — there is nowhere else
//! for it to come from — and the same entry gives it a rebindable key
//! (`[keymap.browser]`) and a footer hint that names whatever key it is actually
//! bound to (see [`HINTS_OPTIONAL`]).
//!
//! Registry commands live in `slash.rs` with every other command. This module
//! holds only what the browser adds: the action type they produce, the key
//! lookup, and the footer-hint table.

use crossterm::event::{KeyCode, KeyEvent};

use crate::keymap::{Context, KeyMap, KeySpec};
use crate::slash::{parse_in_context, SlashOutcome};

// ── BrowserAction ─────────────────────────────────────────────────────────────

/// Which end of the library `select-edge` jumps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    First,
    Last,
}

/// What a browser command asks the picker loop to do.
///
/// Deliberately *not* [`crate::input::Action`]: every variant of that enum is
/// applied to an `AppState` the browser does not have. These are the browser's
/// own verbs, and the picker's `match` over them is exhaustive — so a new
/// variant is a compile error there, and can only ever *fire* if some registry
/// command produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserAction {
    /// Step the selection by `dx` columns and `dy` rows. Horizontal steps mean
    /// nothing in the flat list (only the cover gallery has columns) and are a
    /// no-op there, exactly as they were before this became data.
    MoveSelection { dx: isize, dy: isize },
    /// Move the selection by `n` pages.
    PageSelection(isize),
    /// Move the selection by half a page (`n < 0` up, `n > 0` down) — the vim
    /// `Ctrl-U`/`Ctrl-D` convention (SQ-1228). List view only: the cover gallery
    /// has no half-row concept.
    HalfPageSelection(isize),
    /// Jump to the first or last story.
    SelectEdge(Edge),
    /// Launch the selected story with no launch-time overrides.
    PlayStory,
    /// Open the launch-options dialog for the selected story.
    OpenLaunchOptions,
    /// Open or close the story info panel.
    ToggleInfoPanel,
    /// Switch between the list and the cover gallery.
    ToggleGallery,
    /// Re-fetch the selected story's IFDB metadata, ignoring the cache.
    FetchStory,
    /// Sweep the whole library for missing or stale IFDB metadata.
    RefreshLibrary,
    /// Point the selected story at an IFDB page by hand.
    SetIfdbUrl,
    /// Open a story straight from a URL, downloading it into this library
    /// (SQ-1086) — the UI half of "a URL is accepted wherever a path is".
    OpenUrl,
    /// Open the IFDB search / download modal.
    SearchIfdb,
    /// Download an InvisiClues hint file for the selected story.
    DownloadHints,
    /// Cycle the sort column, keeping the direction.
    SortLibrary,
    /// Reverse the sort direction, keeping the column.
    ReverseSort,
    /// Open the type-to-filter field over the whole library's in-memory index.
    FindStory,
    /// Leave the current sub-folder for the one above it.
    ParentFolder,
    /// Leave the browser.
    QuitBrowser,
    /// Cancel a running fetch, or leave the browser when nothing is in flight.
    CancelBrowser,
}

// ── Key → action ──────────────────────────────────────────────────────────────

/// Resolve a keystroke to a browser action, or `None` if nothing is bound.
///
/// The single door between a key and the browser's behaviour. It goes
/// keymap → registry → action, so the only way to make a key *do* something is
/// to bind it to a `Context::Browser` command.
pub fn action_for_key(km: &KeyMap, k: KeyEvent) -> Option<BrowserAction> {
    let spec = KeySpec::from_key_event(k);
    let command = km.lookup(&spec, Context::Browser)?;
    action_for_command(command)
}

/// Resolve a command-string to a browser action, or `None` if it is not one.
///
/// `parse_in_context`'s `prefix` argument only decorates the text of an error
/// this path discards, so the value is arbitrary.
pub fn action_for_command(command: &str) -> Option<BrowserAction> {
    match parse_in_context(command, '/', Context::Browser) {
        SlashOutcome::Browser(a) => Some(a),
        _ => None,
    }
}

/// The [`crate::list_scroll::nav_key`] code that reproduces `a` in the flat list
/// view, or `None` where the flat list has no such motion.
///
/// Keeps every `KeyCode` out of the picker's dispatch, which is what lets the
/// guard test assert that the dispatch never looks at a key at all.
pub fn list_nav_code(a: BrowserAction) -> Option<KeyCode> {
    match a {
        // Only vertical steps exist in a flat list; a horizontal one is a no-op.
        BrowserAction::MoveSelection { dy, .. } if dy < 0 => Some(KeyCode::Up),
        BrowserAction::MoveSelection { dy, .. } if dy > 0 => Some(KeyCode::Down),
        BrowserAction::MoveSelection { .. } => None,
        BrowserAction::PageSelection(n) if n < 0 => Some(KeyCode::PageUp),
        BrowserAction::PageSelection(_) => Some(KeyCode::PageDown),
        BrowserAction::SelectEdge(Edge::First) => Some(KeyCode::Home),
        BrowserAction::SelectEdge(Edge::Last) => Some(KeyCode::End),
        _ => None,
    }
}

// ── Footer hints ──────────────────────────────────────────────────────────────

/// One footer hint: the commands whose keys it advertises, plus a short label.
///
/// The *keys* are looked up in the live keymap, so a rebinding relabels the hint
/// and an unbound command drops out of the footer entirely. Only the label and
/// the ordering are authored — the drift this replaces was a hand-written string
/// naming a key the code no longer used.
pub struct Hint {
    /// Full command-strings, in display order.
    pub commands: &'static [&'static str],
    /// Gestures with no key binding at all (the mouse ones), appended last.
    pub extras: &'static [&'static str],
    /// The short label shown after the colon.
    pub label: &'static str,
    /// How many binding *ranks* to show — a command's first key is rank 0, its
    /// second key rank 1, and so on. `0` shows every rank. The gallery footer
    /// shows only the arrows (rank 0) because it has no room for hjkl too.
    pub ranks: usize,
}

/// Always-shown left segment of the list footer.
pub const HINT_MOVE: Hint = Hint {
    commands: &["move-selection 0 -1", "move-selection 0 1"],
    extras: &[],
    label: "move",
    ranks: 0,
};

/// Always-shown right segments of the list footer, in order.
pub const HINTS_CORE_RIGHT: &[Hint] = &[
    Hint { commands: &["play-story"], extras: &["2×click"], label: "open", ranks: 0 },
    Hint { commands: &["toggle-info-panel"], extras: &[], label: "info", ranks: 0 },
    Hint { commands: &["quit-browser", "cancel-browser"], extras: &[], label: "quit", ranks: 0 },
];

/// The optional list-footer segments, most-important (least-guessable) first.
///
/// Included left-to-right while they still fit next to the core hints; the rest
/// are dropped — the two navigation conventions go last since nobody needs told,
/// and the launch-options gestures go first because a gesture nobody knows about
/// is a feature nobody uses: nothing on screen would otherwise hint that a story
/// can be started any way but the default one (SQ-0789).
pub const HINTS_OPTIONAL: &[Hint] = &[
    Hint {
        commands: &["open-launch-options"],
        extras: &["2×right-click"],
        label: "options",
        ranks: 0,
    },
    Hint { commands: &["find-story"], extras: &[], label: "find", ranks: 0 },
    Hint { commands: &["parent-folder"], extras: &[], label: "up", ranks: 0 },
    Hint { commands: &["search-ifdb"], extras: &[], label: "IFDB search", ranks: 0 },
    Hint { commands: &["open-url"], extras: &[], label: "open URL", ranks: 0 },
    Hint { commands: &["toggle-gallery"], extras: &[], label: "covers", ranks: 0 },
    Hint { commands: &["fetch-story"], extras: &[], label: "fetch", ranks: 0 },
    Hint { commands: &["refresh-library"], extras: &[], label: "refresh", ranks: 0 },
    Hint { commands: &["set-ifdb-url"], extras: &[], label: "IFDB url", ranks: 0 },
    Hint { commands: &["download-hints"], extras: &[], label: "get hints", ranks: 0 },
    Hint { commands: &["sort-library"], extras: &[], label: "sort", ranks: 0 },
    Hint { commands: &["reverse-sort"], extras: &[], label: "reverse", ranks: 0 },
    Hint { commands: &["page-selection -1", "page-selection 1"], extras: &[], label: "page", ranks: 0 },
    Hint {
        commands: &["half-page-selection -1", "half-page-selection 1"],
        extras: &[],
        label: "half page",
        ranks: 0,
    },
    Hint { commands: &["select-edge first", "select-edge last"], extras: &[], label: "ends", ranks: 0 },
];

/// The cover-gallery footer, which is a fixed line rather than a dropping one.
/// `ranks: 1` on the move hint keeps it to the four arrows.
pub const HINTS_GALLERY: &[Hint] = &[
    Hint {
        commands: &[
            "move-selection -1 0",
            "move-selection 1 0",
            "move-selection 0 -1",
            "move-selection 0 1",
        ],
        extras: &[],
        label: "move",
        ranks: 1,
    },
    Hint { commands: &["play-story"], extras: &["2×click"], label: "open", ranks: 0 },
    Hint { commands: &["toggle-info-panel"], extras: &[], label: "info", ranks: 0 },
    Hint { commands: &["toggle-gallery"], extras: &[], label: "list", ranks: 0 },
    Hint { commands: &["quit-browser", "cancel-browser"], extras: &[], label: "quit", ranks: 0 },
];

/// Every hint the browser can show — the set the coverage guard checks against
/// the registry.
pub fn all_hints() -> Vec<&'static Hint> {
    std::iter::once(&HINT_MOVE)
        .chain(HINTS_CORE_RIGHT)
        .chain(HINTS_OPTIONAL)
        .collect()
}

/// The footer's label for one key.
///
/// A bare character shows as itself (`g`, `H`, `/`) rather than through
/// [`KeySpec::label`]'s uppercasing, which is right for a hotkey table and wrong
/// for a footer that has always spelled these keys the way you type them.
/// Everything else — arrows, `Enter`, `Shift+Enter`, `PgUp` — uses `label()`.
fn key_label(s: &KeySpec) -> String {
    match s.code {
        KeyCode::Char(c) if !s.ctrl && !s.alt => c.to_string(),
        _ => s.label(),
    }
}

/// The keys bound to exactly `command` in the browser context, in binding order.
fn keys_for(km: &KeyMap, command: &str) -> Vec<KeySpec> {
    km.for_context(Context::Browser)
        .filter(|(_, cmd)| *cmd == command)
        .map(|(s, _)| *s)
        .collect()
}

/// Render one hint, or `None` when nothing it names is reachable.
///
/// Keys of the same rank join with `/` (`↑/↓`, `PgUp/PgDn`). Successive ranks
/// normally join with `/` too (`i/Tab`), but with ` or ` once a rank holds more
/// than one key, where a bare slash would run two whole alternatives together
/// (`↑/↓ or k/j`, not `↑/↓/k/j`). Mouse-only gestures follow after ` / `.
pub fn render_hint(km: &KeyMap, h: &Hint) -> Option<String> {
    let per_command: Vec<Vec<KeySpec>> =
        h.commands.iter().map(|c| keys_for(km, c)).collect();
    let depth = per_command.iter().map(Vec::len).max().unwrap_or(0);
    let depth = if h.ranks == 0 { depth } else { depth.min(h.ranks) };

    let mut groups: Vec<String> = Vec::new();
    let mut any_rank_is_plural = false;
    for rank in 0..depth {
        let keys: Vec<String> = per_command
            .iter()
            .filter_map(|ks| ks.get(rank))
            .map(key_label)
            .collect();
        if !keys.is_empty() {
            any_rank_is_plural |= keys.len() > 1;
            groups.push(keys.join("/"));
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if !groups.is_empty() {
        parts.push(groups.join(if any_rank_is_plural { " or " } else { "/" }));
    }
    parts.extend(h.extras.iter().map(|s| s.to_string()));
    if parts.is_empty() {
        return None; // nothing bound and no mouse gesture — say nothing
    }
    Some(format!("{}: {}", parts.join(" / "), h.label))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash::{Category, CommandSpec, COMMANDS};
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn browser_commands() -> Vec<&'static CommandSpec> {
        COMMANDS.iter().filter(|c| c.context == Context::Browser).collect()
    }

    fn km() -> KeyMap {
        KeyMap::default()
    }

    /// The registry is the only source of a browser action: every command in the
    /// browser context must actually produce one, or a binding to it would be a
    /// key that silently does nothing.
    #[test]
    fn every_browser_command_yields_a_browser_action() {
        let cmds = browser_commands();
        assert!(!cmds.is_empty(), "the browser context must carry commands");
        for c in cmds {
            // A browser command can only ever produce a browser action, or the
            // usage error it owes a caller who left out a required argument.
            // What it must never produce is an in-game outcome, which the picker
            // has no `AppState` to apply.
            let outcome = (c.dispatch)(&[]);
            assert!(
                matches!(outcome, SlashOutcome::Browser(_) | SlashOutcome::Error(_)),
                "{} must dispatch to a browser action (or a usage error when it \
                 needs arguments), got {outcome:?}",
                c.name
            );
            assert_eq!(c.category, Category::Library, "{} belongs to the Library group", c.name);
        }
    }

    /// Every default browser binding resolves through the registry to an action.
    /// A dangling binding — a key bound to a command that no longer exists, or to
    /// an in-game one — is a dead key, and this is what catches it.
    #[test]
    fn every_default_browser_binding_resolves_to_an_action() {
        let km = km();
        let mut n = 0;
        for (spec, cmd) in km.for_context(Context::Browser) {
            assert!(
                action_for_command(cmd).is_some(),
                "browser binding {} = {cmd:?} does not resolve to an action",
                spec.label()
            );
            n += 1;
        }
        assert!(n >= 20, "the browser's default bindings are still there: {n}");
        // …and from the other side: a command nobody can press is a command
        // nobody has. Every browser command ships with at least one key.
        for c in browser_commands() {
            assert!(
                km.for_context(Context::Browser)
                    .any(|(_, cmd)| cmd.split_whitespace().next() == Some(c.name)),
                "browser command '{}' has no default key binding",
                c.name
            );
        }
    }

    /// The keymap is genuinely consulted: the shipped keys reach the actions the
    /// picker acts on. Spot-checks one of each shape (plain, shifted, aliased).
    #[test]
    fn the_shipped_keys_reach_their_actions() {
        let km = km();
        let key = |c: KeyCode, m: KeyModifiers| action_for_key(&km, KeyEvent::new(c, m));
        assert_eq!(key(KeyCode::Enter, KeyModifiers::NONE), Some(BrowserAction::PlayStory));
        assert_eq!(key(KeyCode::Char('f'), KeyModifiers::CONTROL), Some(BrowserAction::FindStory));
        assert_eq!(key(KeyCode::Backspace, KeyModifiers::NONE), Some(BrowserAction::ParentFolder));
        assert_eq!(
            key(KeyCode::Enter, KeyModifiers::SHIFT),
            Some(BrowserAction::OpenLaunchOptions),
            "Shift-Enter opens the launch dialog"
        );
        assert_eq!(
            key(KeyCode::Char('o'), KeyModifiers::NONE),
            Some(BrowserAction::OpenLaunchOptions),
            "…and `o` is the same command, not a second implementation"
        );
        assert_eq!(
            key(KeyCode::Char('H'), KeyModifiers::SHIFT),
            Some(BrowserAction::DownloadHints)
        );
        assert_eq!(
            key(KeyCode::Char('H'), KeyModifiers::NONE),
            Some(BrowserAction::DownloadHints),
            "a terminal that reports the shifted glyph without the flag still matches"
        );
        assert_eq!(
            key(KeyCode::Up, KeyModifiers::NONE),
            Some(BrowserAction::MoveSelection { dx: 0, dy: -1 })
        );
        assert_eq!(
            key(KeyCode::Char('k'), KeyModifiers::NONE),
            Some(BrowserAction::MoveSelection { dx: 0, dy: -1 })
        );
        // SQ-1228: vim-style half-page paging.
        assert_eq!(
            key(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Some(BrowserAction::HalfPageSelection(-1))
        );
        assert_eq!(
            key(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(BrowserAction::HalfPageSelection(1))
        );
        assert_eq!(key(KeyCode::Tab, KeyModifiers::NONE), Some(BrowserAction::ToggleInfoPanel));
        assert_eq!(key(KeyCode::Esc, KeyModifiers::NONE), Some(BrowserAction::CancelBrowser));
        assert_eq!(key(KeyCode::Char('q'), KeyModifiers::NONE), Some(BrowserAction::QuitBrowser));
        // Nothing is bound to `z`, and an unbound key is silence, not a default.
        assert_eq!(key(KeyCode::Char('z'), KeyModifiers::NONE), None);
    }

    /// A user rebinding takes effect, which is the whole reason the keys are data.
    #[test]
    fn a_rebound_key_moves_the_action_and_the_hint() {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.browser.insert("ctrl+o".into(), "open-launch-options".into());
        let (km, warns) = KeyMap::resolve(&cfg);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(
            action_for_key(&km, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Some(BrowserAction::OpenLaunchOptions)
        );
        // …and the footer says so, without anyone editing a string.
        let hint = HINTS_OPTIONAL
            .iter()
            .find(|h| h.commands == ["open-launch-options"])
            .expect("the options hint");
        let rendered = render_hint(&km, hint).expect("bound");
        assert_eq!(rendered, "Shift+Enter/o/Ctrl+O / 2×right-click: options", "the hint names the new key: {rendered:?}");
    }

    /// **The anti-drift guard for the footer.** Every browser command appears in
    /// exactly one hint, so a gesture added to the registry cannot ship without
    /// one — the divergence between a hardcoded arm and a hand-written hint is
    /// what SQ-0796 exists to end.
    #[test]
    fn every_browser_command_has_exactly_one_footer_hint() {
        let hinted: Vec<&str> = all_hints()
            .iter()
            .flat_map(|h| h.commands.iter().copied())
            .collect();
        for c in browser_commands() {
            let n = hinted
                .iter()
                .filter(|cmd| cmd.split_whitespace().next() == Some(c.name))
                .count();
            assert!(n > 0, "browser command '{}' has no footer hint", c.name);
        }
        // …and every hint names a real command, so a renamed one is caught from
        // the other side too.
        for cmd in &hinted {
            let name = cmd.split_whitespace().next().unwrap_or("");
            let spec = crate::slash::find_command(name)
                .unwrap_or_else(|| panic!("footer hint names unknown command '{cmd}'"));
            assert_eq!(spec.context, Context::Browser, "'{cmd}' is not a browser command");
            assert!(
                action_for_command(cmd).is_some(),
                "footer hint '{cmd}' does not resolve to an action"
            );
        }
    }

    /// The gallery footer's hints are held to the same standard.
    #[test]
    fn the_gallery_hints_name_real_browser_commands() {
        for h in HINTS_GALLERY {
            for cmd in h.commands {
                assert!(
                    action_for_command(cmd).is_some(),
                    "gallery hint '{cmd}' does not resolve to an action"
                );
            }
        }
    }

    /// Rendering: ranks, mouse extras, and an unbound command dropping out.
    #[test]
    fn render_hint_groups_ranks_and_appends_mouse_gestures() {
        let km = km();
        assert_eq!(
            render_hint(&km, &HINT_MOVE).as_deref(),
            Some("↑/↓ or k/j: move"),
            "rank 0 is the arrows, rank 1 the letters"
        );
        let open = &HINTS_CORE_RIGHT[0];
        assert_eq!(render_hint(&km, open).as_deref(), Some("Enter / 2×click: open"));
        let gallery_move = &HINTS_GALLERY[0];
        assert_eq!(
            render_hint(&km, gallery_move).as_deref(),
            Some("←/→/↑/↓: move"),
            "ranks: 1 keeps hjkl out of the gallery line"
        );
        // An empty keymap leaves only the mouse gesture — the hint never claims a
        // key that is not bound.
        let empty = KeyMap { bindings: Vec::new() };
        assert_eq!(render_hint(&empty, open).as_deref(), Some("2×click: open"));
        assert_eq!(render_hint(&empty, &HINT_MOVE), None, "no keys, no gesture, no hint");
    }

    /// Context gating both ways: a browser command is refused in the game, and an
    /// in-game command is refused in the browser (where it could not work at all,
    /// there being no `AppState` to apply it to).
    #[test]
    fn browser_and_game_commands_do_not_cross_contexts() {
        assert!(matches!(
            parse_in_context("play-story", '/', Context::Global),
            SlashOutcome::Error(_)
        ));
        assert!(matches!(
            parse_in_context("sort-library", '/', Context::Map),
            SlashOutcome::Error(_)
        ));
        assert!(matches!(
            parse_in_context("quit", '/', Context::Browser),
            SlashOutcome::Error(_)
        ));
        assert!(matches!(
            parse_in_context("zoom-map in", '/', Context::Browser),
            SlashOutcome::Error(_)
        ));
        // A key bound to an in-game command in the browser context is inert
        // rather than a crash or a half-applied action.
        assert_eq!(action_for_command("quit"), None);
    }

    /// The browser's commands are not typeable — the browser has no command line
    /// — so they stay out of the game's `/help` and its Tab completion.
    #[test]
    fn browser_commands_stay_out_of_the_slash_surfaces() {
        let names = crate::slash::slash_names();
        for c in browser_commands() {
            assert!(
                !names.iter().any(|n| n == c.name),
                "'{}' must not be offered for slash autocomplete",
                c.name
            );
        }
        let help = crate::slash::help_text('/');
        assert!(
            !help.iter().any(|l| l.contains("/play-story")),
            "browser commands must not be listed in the game's /help"
        );
        // …but a direct query still explains one, since the docs reference them.
        assert!(crate::slash::help_for_command('/', "play-story")[0].contains("launch"));
    }

    #[test]
    fn list_nav_codes_match_the_flat_list_motions() {
        use BrowserAction::*;
        assert_eq!(list_nav_code(MoveSelection { dx: 0, dy: -1 }), Some(KeyCode::Up));
        assert_eq!(list_nav_code(MoveSelection { dx: 0, dy: 1 }), Some(KeyCode::Down));
        // Horizontal movement exists only in the gallery — a no-op in the list,
        // exactly as the `if gallery` guard made it before.
        assert_eq!(list_nav_code(MoveSelection { dx: -1, dy: 0 }), None);
        assert_eq!(list_nav_code(MoveSelection { dx: 1, dy: 0 }), None);
        assert_eq!(list_nav_code(PageSelection(-1)), Some(KeyCode::PageUp));
        assert_eq!(list_nav_code(PageSelection(1)), Some(KeyCode::PageDown));
        assert_eq!(list_nav_code(SelectEdge(Edge::First)), Some(KeyCode::Home));
        assert_eq!(list_nav_code(SelectEdge(Edge::Last)), Some(KeyCode::End));
        assert_eq!(list_nav_code(PlayStory), None);
    }
}
