//! The story browser's per-story menu (SQ-1227).
//!
//! A small popup anchored beside the highlighted row (or gallery tile), in the
//! shape a desktop context menu takes: the actions that apply to *this* story,
//! each with its own hotkey shown right-aligned for reference.
//!
//! It exists because the footer could not hold them. Five of the browser's
//! gestures — launch options, fetch, hints, the manual IFDB URL — act on one
//! story and one story only, and advertising each of them cost a footer segment
//! that was the first thing dropped on any terminal narrower than a page. One
//! key (`Space`, or a right-click) now advertises all five, and the footer is
//! left to say what the LIBRARY does.
//!
//! Every item dispatches an existing `slash::COMMANDS` entry, and its key column
//! is read from the live keymap — so the menu can no more invent a gesture than
//! the footer can, and rebinding one relabels the row (`browser::first_key`).
//!
//! The chrome, the key handling and the geometry are `crate::menu`'s (SQ-1265):
//! this file supplies only the item list, which `Context` their keys live in,
//! and the bookkeeping of which story the menu was opened for.

use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::browser::first_key;
use crate::colors::ColorScheme;
use crate::keymap::KeyMap;
use crate::menu as menu_widget;

pub use crate::menu::{MenuItem, MenuOutcome, MenuRects};

// ── The items ─────────────────────────────────────────────────────────────────

/// The menu, top to bottom. `Open` leads because it is what the row does
/// anyway; the rest are the per-story gestures the footer used to carry.
pub const STORY_MENU: &[MenuItem] = &[
    MenuItem { command: "play-story", label: "Open" },
    MenuItem { command: "open-launch-options", label: "Launch options…" },
    MenuItem { command: "fetch-story", label: "Fetch metadata" },
    MenuItem { command: "download-hints", label: "Get hints" },
    MenuItem { command: "set-ifdb-url", label: "Set IFDB URL…" },
];

// ── State ─────────────────────────────────────────────────────────────────────

/// The open menu: which story it belongs to, and which row the cursor is on.
#[derive(Debug, Clone, Copy)]
pub struct StoryMenu {
    /// The list index the menu was opened for. The picker selects that row
    /// before opening, so this is the selection — kept so a redraw can anchor
    /// the popup on the row even after the list scrolls under it.
    pub story: usize,
    /// The highlighted item.
    pub cursor: usize,
}

impl StoryMenu {
    /// Open the menu on `story`, cursor on the first item.
    pub fn new(story: usize) -> StoryMenu {
        StoryMenu { story, cursor: 0 }
    }

    /// Route a keystroke. See [`crate::menu::on_key`].
    pub fn on_key(&mut self, k: KeyEvent, km: &KeyMap) -> MenuOutcome {
        menu_widget::on_key(&mut self.cursor, STORY_MENU, k, km, first_key)
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────────

/// The menu's frame for an anchor row, clamped inside `pane`. See
/// [`crate::menu::menu_rect`].
pub fn menu_rect(km: &KeyMap, anchor: Rect, pane: Rect) -> Rect {
    menu_widget::menu_rect(STORY_MENU, km, first_key, anchor, pane)
}

// ── Draw ──────────────────────────────────────────────────────────────────────

/// Draw the menu over the list, anchored beside `anchor` and clamped to `pane`.
/// See [`crate::menu::draw_menu`].
pub fn draw_story_menu(
    menu: &StoryMenu,
    anchor: Rect,
    pane: Rect,
    km: &KeyMap,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> MenuRects {
    menu_widget::draw_menu(STORY_MENU, menu.cursor, km, first_key, anchor, pane, cs, buf)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn km() -> KeyMap {
        KeyMap::default()
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    /// Every item runs a real browser command that the shipped keymap can
    /// actually reach — a menu row whose key column is blank teaches nothing.
    #[test]
    fn every_menu_item_names_a_bound_browser_command() {
        let km = km();
        for it in STORY_MENU {
            let spec = crate::slash::find_command(it.command)
                .unwrap_or_else(|| panic!("story menu names unknown command '{}'", it.command));
            assert_eq!(spec.context, crate::keymap::Context::Browser, "{}", it.command);
            assert!(
                first_key(&km, it.command).is_some(),
                "story menu item '{}' has no default browser key",
                it.label
            );
        }
    }

    #[test]
    fn the_cursor_wraps_and_enter_activates() {
        let km = km();
        let mut m = StoryMenu::new(3);
        assert_eq!(m.on_key(key(KeyCode::Up), &km), MenuOutcome::None);
        assert_eq!(m.cursor, STORY_MENU.len() - 1, "Up from the top wraps to the bottom");
        assert_eq!(m.on_key(key(KeyCode::Down), &km), MenuOutcome::None);
        assert_eq!(m.cursor, 0);
        m.cursor = 1;
        assert_eq!(
            m.on_key(key(KeyCode::Enter), &km),
            MenuOutcome::Activate("open-launch-options")
        );
        assert_eq!(m.on_key(key(KeyCode::Esc), &km), MenuOutcome::Close);
    }

    /// An item's own hotkey activates it wherever the cursor is — the menu
    /// teaches the key and then gets out of the way.
    #[test]
    fn an_items_own_hotkey_activates_it_directly() {
        let km = km();
        let mut m = StoryMenu::new(0);
        assert_eq!(m.on_key(key(KeyCode::Char('f')), &km), MenuOutcome::Activate("fetch-story"));
        assert_eq!(m.on_key(key(KeyCode::Char('u')), &km), MenuOutcome::Activate("set-ifdb-url"));
        assert_eq!(
            m.on_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT), &km),
            MenuOutcome::Activate("download-hints")
        );
        // A key nothing here is bound to is swallowed, not acted on.
        assert_eq!(m.on_key(key(KeyCode::Char('z')), &km), MenuOutcome::None);
    }

    /// The key column says what the keymap says, not what a string says.
    #[test]
    fn the_key_column_reads_the_keymap() {
        assert_eq!(
            menu_widget::key_labels(STORY_MENU, &km(), &first_key),
            vec!["Enter", "o", "f", "Shift+H", "u"]
        );
    }

    #[test]
    fn the_menu_is_clamped_inside_the_pane() {
        let km = km();
        let pane = Rect::new(0, 0, 40, 20);
        // A row near the bottom flips the menu above itself.
        let low = menu_rect(&km, Rect::new(2, 18, 30, 1), pane);
        assert!(low.bottom() <= pane.bottom(), "{low:?}");
        assert!(low.y < 18, "a row with no room below is served from above: {low:?}");
        // …and a row at the very right edge is pulled back inside.
        let far = menu_rect(&km, Rect::new(38, 2, 2, 1), pane);
        assert!(far.right() <= pane.right(), "{far:?}");
        assert!(far.x >= pane.x);
        // The usual case: below the row, aligned with it.
        let normal = menu_rect(&km, Rect::new(2, 3, 30, 1), pane);
        assert_eq!((normal.x, normal.y), (2, 4));
        assert_eq!(normal.height, STORY_MENU.len() as u16 + 2);
    }

    #[test]
    fn the_menu_draws_its_labels_and_keys() {
        let km = km();
        let cs = ColorScheme::terminal_default();
        let pane = Rect::new(0, 0, 60, 24);
        let mut buf = Buffer::empty(pane);
        let menu = StoryMenu::new(0);
        let rects = draw_story_menu(&menu, Rect::new(1, 2, 40, 1), pane, &km, &cs, &mut buf);
        assert_eq!(rects.items.len(), STORY_MENU.len());
        let text: String = (rects.area.y..rects.area.bottom())
            .map(|y| {
                (rects.area.x..rects.area.right())
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // A context menu, not a dialog: a bordered box with no title strip
        // notched into its top edge, labels left and keys in their own column.
        assert_eq!(
            text,
            "┌──────────────────────────┐\n\
             │ Open             Enter   │\n\
             │ Launch options…  o       │\n\
             │ Fetch metadata   f       │\n\
             │ Get hints        Shift+H │\n\
             │ Set IFDB URL…    u       │\n\
             └──────────────────────────┘",
            "{text}"
        );
    }
}
