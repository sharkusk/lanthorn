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
    /// Open the per-story menu beside the highlighted row or tile (SQ-1227).
    OpenStoryMenu,
    /// Show the browser's own key reference (SQ-1227). Its own dialog rather
    /// than the game's hotkey panel, which lists in-game commands and is fed
    /// from an `AppState` the browser does not have.
    ShowBrowserKeys,
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

/// One footer hint: a command and the short label shown after its key.
///
/// **One key per hint** (SQ-1227). The key is the FIRST one bound to `command`
/// in the browser context, looked up in the live keymap — so a rebinding
/// relabels the hint and an unbound command drops out of the footer entirely —
/// and the alternates (`i`, `Esc`, `Shift+Enter`, `k`/`j`, …) stay bound but
/// unadvertised. The footer is a legend of the LIBRARY-level keys, not an
/// inventory of every route to every gesture: mouse gestures are never in it,
/// navigation is never in it, and everything that acts on ONE story lives in the
/// story menu instead (`crate::story_menu`).
#[derive(Clone, Copy, Debug)]
pub struct Hint {
    /// The full command-string whose key this hint names.
    pub command: &'static str,
    /// The short label shown after the colon.
    pub label: &'static str,
    /// Drop priority when the footer will not fit: the LOWEST rank is dropped
    /// first, and `None` is never dropped at all.
    pub drop_rank: Option<u8>,
}

/// The footer, in fixed left-to-right display order (SQ-1227):
///
/// ```text
/// Enter: open  Space: menu  Tab: info  /: IFDB  g: covers  s: sort  r: refresh  Ctrl+F: find  ?: keys  q: quit
/// ```
///
/// `drop_rank` is the order they go as the terminal narrows — `find` first,
/// then `refresh`, `sort`, `covers`, `IFDB`, `info`, and `keys` last. Open, menu
/// and quit carry no rank and are always shown: without the first two there is
/// no way to learn anything else, and without the third no way out.
///
/// What is NOT here is the point of the table. Navigation is gone (nobody needs
/// told that ↑ moves), the mouse is gone, and every gesture that acts on one
/// STORY — launch options, fetch, hints, the IFDB URL — moved to the story menu
/// behind `Space`, which is one key to advertise instead of five.
pub const HINTS: &[Hint] = &[
    Hint { command: "play-story", label: "open", drop_rank: None },
    Hint { command: "open-story-menu", label: "menu", drop_rank: None },
    Hint { command: "toggle-info-panel", label: "info", drop_rank: Some(5) },
    Hint { command: "search-ifdb", label: "IFDB", drop_rank: Some(4) },
    Hint { command: "toggle-gallery", label: "covers", drop_rank: Some(3) },
    Hint { command: "sort-library", label: "sort", drop_rank: Some(2) },
    Hint { command: "refresh-library", label: "refresh", drop_rank: Some(1) },
    Hint { command: "find-story", label: "find", drop_rank: Some(0) },
    Hint { command: "show-browser-keys", label: "keys", drop_rank: Some(6) },
    Hint { command: "quit-browser", label: "quit", drop_rank: None },
];

/// The footer for one view. Identical in both, except that in the cover gallery
/// the `toggle-gallery` hint names where the key GOES (back to the `list`)
/// rather than where it is.
pub fn footer_hints(gallery: bool) -> Vec<Hint> {
    HINTS
        .iter()
        .map(|h| {
            if gallery && h.command == "toggle-gallery" {
                Hint { label: "list", ..*h }
            } else {
                *h
            }
        })
        .collect()
}

/// The label for one key, in the footer and in the story menu's key column.
///
/// A bare unshifted character shows as itself (`g`, `/`, `?`) rather than
/// through [`KeySpec::label`]'s uppercasing, which is right for a hotkey table
/// and wrong for a footer that has always spelled these keys the way you type
/// them. Everything else — `Space`, arrows, `Enter`, `Shift+H`, `Ctrl+F` — uses
/// `label()`, so a chord is always spelled as a chord.
pub fn key_label(s: &KeySpec) -> String {
    match s.code {
        // `Char(' ')` is a key you press, not a character you type.
        KeyCode::Char(' ') => s.label(),
        KeyCode::Char(c) if !s.ctrl && !s.alt && !s.shift => c.to_string(),
        _ => s.label(),
    }
}

/// The FIRST key bound to exactly `command` in the browser context.
///
/// "First" is binding order, authored in `keymap.rs` precisely so that the key a
/// hint or a menu row names is the one worth telling somebody about — and a
/// `[keymap.browser]` line that reuses a default's key displaces it there, so a
/// genuine rebinding moves the label with it.
pub fn first_key(km: &KeyMap, command: &str) -> Option<KeySpec> {
    km.for_context(Context::Browser).find(|(_, cmd)| *cmd == command).map(|(s, _)| *s)
}

/// Render one hint, or `None` when nothing is bound to its command.
pub fn render_hint(km: &KeyMap, h: &Hint) -> Option<String> {
    let key = first_key(km, h.command)?;
    Some(format!("{}: {}", key_label(&key), h.label))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-input"))]
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
        // SQ-1227: the per-story menu and the key reference.
        assert_eq!(key(KeyCode::Char(' '), KeyModifiers::NONE), Some(BrowserAction::OpenStoryMenu));
        assert_eq!(
            key(KeyCode::Char('?'), KeyModifiers::NONE),
            Some(BrowserAction::ShowBrowserKeys)
        );
        assert_eq!(
            key(KeyCode::Char('?'), KeyModifiers::SHIFT),
            Some(BrowserAction::ShowBrowserKeys),
            "a terminal that reports `?` with the shift flag still matches"
        );
        assert_eq!(key(KeyCode::Esc, KeyModifiers::NONE), Some(BrowserAction::CancelBrowser));
        assert_eq!(key(KeyCode::Char('q'), KeyModifiers::NONE), Some(BrowserAction::QuitBrowser));
        // Nothing is bound to `z`, and an unbound key is silence, not a default.
        assert_eq!(key(KeyCode::Char('z'), KeyModifiers::NONE), None);
    }

    /// A user rebinding takes effect, which is the whole reason the keys are data.
    #[test]
    fn a_rebound_key_moves_the_action_and_the_hint() {
        let mut cfg = crate::config::KeymapConfig::default();
        // `x` takes the command and `s` is given away, so the DEFAULT binding is
        // displaced rather than merely joined — which is what "rebound" means.
        cfg.browser.insert("s".into(), "reverse-sort".into());
        cfg.browser.insert("x".into(), "sort-library".into());
        let (km, warns) = KeyMap::resolve(&cfg);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(
            action_for_key(&km, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(BrowserAction::SortLibrary)
        );
        // …and the footer says so, without anyone editing a string.
        let hint = HINTS.iter().find(|h| h.command == "sort-library").expect("the sort hint");
        assert_eq!(render_hint(&km, hint).as_deref(), Some("x: sort"));
    }

    /// **The anti-drift guard for the footer and the menu.** Both are tables of
    /// command NAMES, and a name that no longer resolves is a hint or a row that
    /// silently does nothing — the divergence between a hardcoded arm and a
    /// hand-written label is what SQ-0796 exists to end.
    ///
    /// The menu carries the stronger half: its rows show a key column read from
    /// the keymap, so an item whose command ships with no default binding would
    /// draw a blank one.
    #[test]
    fn every_footer_hint_and_menu_item_names_a_real_command() {
        let km = km();
        for h in HINTS {
            let spec = crate::slash::find_command(h.command)
                .unwrap_or_else(|| panic!("footer hint names unknown command '{}'", h.command));
            assert_eq!(spec.context, Context::Browser, "'{}' is not a browser command", h.command);
            assert!(
                action_for_command(h.command).is_some(),
                "footer hint '{}' does not resolve to an action",
                h.command
            );
        }
        for it in crate::story_menu::STORY_MENU {
            let spec = crate::slash::find_command(it.command)
                .unwrap_or_else(|| panic!("story menu names unknown command '{}'", it.command));
            assert_eq!(spec.context, Context::Browser, "'{}' is not a browser command", it.command);
            assert!(
                action_for_command(it.command).is_some(),
                "story menu item '{}' does not resolve to an action",
                it.command
            );
            assert!(
                first_key(&km, it.command).is_some(),
                "story menu item '{}' has no default browser binding, so its key \
                 column would be blank",
                it.command
            );
        }
    }

    /// The footer says exactly what SQ-1227 specified, in that order, one key
    /// each — and the gallery says the same with `g` naming where it goes.
    #[test]
    fn the_footer_is_one_key_per_hint() {
        let km = km();
        let line: Vec<String> =
            footer_hints(false).iter().filter_map(|h| render_hint(&km, h)).collect();
        assert_eq!(
            line,
            vec![
                "Enter: open",
                "Space: menu",
                "Tab: info",
                "/: IFDB",
                "g: covers",
                "s: sort",
                "r: refresh",
                "Ctrl+F: find",
                "?: keys",
                "q: quit",
            ]
        );
        let gallery: Vec<String> =
            footer_hints(true).iter().filter_map(|h| render_hint(&km, h)).collect();
        assert!(gallery.contains(&"g: list".to_string()), "{gallery:?}");
        assert_eq!(gallery.len(), line.len(), "the two footers carry the same hints");
    }

    /// One key per hint even where several are bound: `i` and `Esc` stay live,
    /// and stay out of the footer.
    #[test]
    fn a_hint_names_one_key_however_many_are_bound() {
        let km = km();
        let info = HINTS.iter().find(|h| h.command == "toggle-info-panel").expect("info");
        assert_eq!(render_hint(&km, info).as_deref(), Some("Tab: info"), "not `i/Tab`");
        assert_eq!(
            action_for_key(&km, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
            Some(BrowserAction::ToggleInfoPanel),
            "…and `i` is still bound, just unadvertised"
        );
        let quit = HINTS.iter().find(|h| h.command == "quit-browser").expect("quit");
        assert_eq!(render_hint(&km, quit).as_deref(), Some("q: quit"), "not `q/Esc`");
        assert_eq!(
            action_for_key(&km, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(BrowserAction::CancelBrowser)
        );

        // A user's EXTRA binding does not lengthen the hint either.
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.browser.insert("ctrl+g".into(), "toggle-gallery".into());
        let (km2, warns) = KeyMap::resolve(&cfg);
        assert!(warns.is_empty(), "{warns:?}");
        let covers = HINTS.iter().find(|h| h.command == "toggle-gallery").expect("covers");
        assert_eq!(render_hint(&km2, covers).as_deref(), Some("g: covers"));
    }

    /// A hint whose command nobody has bound is not shown — the footer has no
    /// way to claim a key that does not exist.
    #[test]
    fn an_unbound_command_has_no_hint() {
        let empty = KeyMap { bindings: Vec::new() };
        for h in HINTS {
            assert_eq!(render_hint(&empty, h), None, "{}", h.command);
        }
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
