//! The room panel's right-click context menu (SQ-1265).
//!
//! Mirrors `story_menu`'s per-story popup for the map: right-click a room and
//! get the gestures that act on it — rename it, move its region, rename its
//! layer — anchored at the click, instead of the two-key hunt through the
//! leader (`Ctrl+P`) dialog. Chrome, key handling and geometry are
//! `crate::menu`'s (SQ-1265); this file supplies the item list, the
//! `Context::Map` key lookup, and which room the menu is for.
//!
//! Right-clicking the room to open this menu also pins the room dock on it —
//! see `input::apply_action`'s `Action::OpenRoomMenu` — so the menu and the
//! panel underneath always agree on which room is meant.

use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use mapper::graph::RoomId;

use crate::colors::ColorScheme;
use crate::keymap::{Context, KeyMap, KeySpec};
use crate::menu as menu_widget;

pub use crate::menu::{MenuItem, MenuOutcome, MenuRects};

/// The FIRST key bound to `command` in `Context::Map` — the room menu's own
/// key lookup, the way `browser::first_key` is the story menu's.
fn first_key(km: &KeyMap, command: &str) -> Option<KeySpec> {
    km.first_key(Context::Map, command)
}

/// The menu, top to bottom. Each item dispatches an existing `slash::COMMANDS`
/// entry, bare: `rename-room` and `move-region` act on `state.selected_room`,
/// which the click that opens the menu has already pinned; `rename-layer`
/// acts on the active layer (`AppState::active_layer`), which is whatever
/// layer the map is showing — the one the clicked room is visibly on.
pub const ROOM_MENU: &[MenuItem] = &[
    MenuItem { command: "rename-room", label: "Rename Room" },
    MenuItem { command: "move-region", label: "Move Region" },
    MenuItem { command: "rename-layer", label: "Rename Layer" },
];

/// The open menu: which room it belongs to, where it is anchored, and which
/// row the cursor is on.
#[derive(Debug, Clone, Copy)]
pub struct RoomMenu {
    /// The room the menu was opened for — kept the way `StoryMenu::story`
    /// keeps the list index, so the panel underneath and the menu's own
    /// dispatch agree on which room is meant.
    pub room: RoomId,
    /// The terminal cell the menu was opened at (the right-click's column and
    /// row). Unlike the story menu's list row, a room has no live layout
    /// rect to re-derive this from each frame, and nothing can move the map
    /// out from under it while the menu owns the mouse and keyboard, so a
    /// fixed point is exact rather than an approximation.
    pub anchor: (u16, u16),
    /// The highlighted item.
    pub cursor: usize,
}

impl RoomMenu {
    /// Open the menu on `room`, anchored at the click, cursor on the first item.
    pub fn new(room: RoomId, col: u16, row: u16) -> RoomMenu {
        RoomMenu { room, anchor: (col, row), cursor: 0 }
    }

    /// Route a keystroke. See [`crate::menu::on_key`].
    pub fn on_key(&mut self, k: KeyEvent, km: &KeyMap) -> MenuOutcome {
        menu_widget::on_key(&mut self.cursor, ROOM_MENU, k, km, first_key)
    }

    /// The 1x1 rect `crate::menu`'s geometry anchors against.
    fn anchor_rect(&self) -> Rect {
        Rect::new(self.anchor.0, self.anchor.1, 1, 1)
    }
}

/// The menu's frame, clamped inside `pane` (the map pane). See
/// [`crate::menu::menu_rect`].
pub fn menu_rect(menu: &RoomMenu, km: &KeyMap, pane: Rect) -> Rect {
    menu_widget::menu_rect(ROOM_MENU, km, first_key, menu.anchor_rect(), pane)
}

/// Draw the menu anchored at the click, clamped to `pane`. See
/// [`crate::menu::draw_menu`].
pub fn draw_room_menu(
    menu: &RoomMenu,
    pane: Rect,
    km: &KeyMap,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> MenuRects {
    menu_widget::draw_menu(ROOM_MENU, menu.cursor, km, first_key, menu.anchor_rect(), pane, cs, buf)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-input"))]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn km() -> KeyMap {
        KeyMap::default()
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    /// Every item names a real `Context::Map` command — a menu that could
    /// dispatch an unregistered string would panic the first time somebody
    /// clicked it.
    #[test]
    fn every_menu_item_names_a_map_command() {
        for it in ROOM_MENU {
            let spec = crate::slash::find_command(it.command)
                .unwrap_or_else(|| panic!("room menu names unknown command '{}'", it.command));
            assert_eq!(spec.context, Context::Map, "{}", it.command);
        }
    }

    #[test]
    fn the_cursor_wraps_and_enter_activates() {
        let km = km();
        let mut m = RoomMenu::new(3, 5, 5);
        assert_eq!(m.on_key(key(KeyCode::Up), &km), MenuOutcome::None);
        assert_eq!(m.cursor, ROOM_MENU.len() - 1, "Up from the top wraps to the bottom");
        assert_eq!(m.on_key(key(KeyCode::Down), &km), MenuOutcome::None);
        assert_eq!(m.cursor, 0);
        m.cursor = 1;
        assert_eq!(m.on_key(key(KeyCode::Enter), &km), MenuOutcome::Activate("move-region"));
        assert_eq!(m.on_key(key(KeyCode::Esc), &km), MenuOutcome::Close);
    }

    /// None of the three ships with a bare-key binding (all three live only in
    /// the leader/`Ctrl+P` dialog, a wholly separate table — `HotkeyLayout`,
    /// not `KeyMap`), so the key column is blank for every row, and a key that
    /// matches none of them is swallowed rather than acted on.
    #[test]
    fn no_default_hotkeys_and_an_unbound_key_is_swallowed() {
        let km = km();
        assert!(
            ROOM_MENU.iter().all(|it| first_key(&km, it.command).is_none()),
            "none of these are bound outside the leader dialog",
        );
        let mut m = RoomMenu::new(0, 5, 5);
        assert_eq!(m.on_key(key(KeyCode::Char('r')), &km), MenuOutcome::None);
    }

    #[test]
    fn the_menu_is_clamped_inside_the_pane() {
        let km = km();
        let pane = Rect::new(0, 0, 40, 20);
        let low = menu_rect(&RoomMenu::new(0, 2, 18), &km, pane);
        assert!(low.bottom() <= pane.bottom(), "{low:?}");
        assert!(low.y < 18, "a click with no room below is served from above: {low:?}");
        let far = menu_rect(&RoomMenu::new(0, 38, 2), &km, pane);
        assert!(far.right() <= pane.right(), "{far:?}");
        assert!(far.x >= pane.x);
        let normal = menu_rect(&RoomMenu::new(0, 2, 3), &km, pane);
        assert_eq!((normal.x, normal.y), (2, 4));
        assert_eq!(normal.height, ROOM_MENU.len() as u16 + 2);
    }

    /// The menu wears the SAME chrome as the story menu — same border glyphs,
    /// same panel primitive — because it reuses `dialog.story_menu.*` rather
    /// than a selector of its own (SQ-1265): one popup-menu look, themed once.
    #[test]
    fn the_menu_draws_the_shared_chrome_and_labels() {
        let km = km();
        let cs = ColorScheme::terminal_default();
        let pane = Rect::new(0, 0, 60, 24);
        let mut buf = Buffer::empty(pane);
        let menu = RoomMenu::new(1, 5, 5);
        let rects = draw_room_menu(&menu, pane, &km, &cs, &mut buf);
        assert_eq!(rects.items.len(), ROOM_MENU.len());

        let text: String = (rects.area.y..rects.area.bottom())
            .map(|y| {
                (rects.area.x..rects.area.right())
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The same bordered box with no title strip the story menu draws, and
        // all three labels present — the key column is empty (no defaults, see
        // above), so it is not asserted here.
        assert!(text.starts_with('┌') && text.contains('┐'), "shared border glyphs: {text:?}");
        assert!(text.contains("Rename Room"), "{text}");
        assert!(text.contains("Move Region"), "{text}");
        assert!(text.contains("Rename Layer"), "{text}");
    }
}
