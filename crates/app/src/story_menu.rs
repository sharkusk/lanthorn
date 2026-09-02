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

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::browser::{first_key, key_label};
use crate::colors::ColorScheme;
use crate::keymap::{KeySpec, KeyMap};
use crate::render::draw_str_clipped;
use crate::render::paneframe::BorderStyle;
use crate::render::panel::{draw_panel, PanelSpec};

// ── The items ─────────────────────────────────────────────────────────────────

/// One menu row: the registry command it runs, and what it is called here.
pub struct MenuItem {
    /// A full command-string in `Context::Browser`, dispatched exactly as a key
    /// bound to it would be.
    pub command: &'static str,
    /// The row's text. A trailing `…` is the usual promise that the item opens
    /// something to answer rather than acting straight away.
    pub label: &'static str,
}

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

/// What a keystroke or a click did to the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOutcome {
    /// Handled inside the menu (the cursor moved, or the key means nothing here).
    None,
    /// Dismiss without acting.
    Close,
    /// Run this command-string and dismiss.
    Activate(&'static str),
}

impl StoryMenu {
    /// Open the menu on `story`, cursor on the first item.
    pub fn new(story: usize) -> StoryMenu {
        StoryMenu { story, cursor: 0 }
    }

    /// Route a keystroke.
    ///
    /// Up/Down wrap, Enter activates, Esc closes — and an item's OWN hotkey
    /// activates it directly, so the menu never gets in the way of somebody who
    /// already knows the key it is teaching them.
    pub fn on_key(&mut self, k: KeyEvent, km: &KeyMap) -> MenuOutcome {
        let n = STORY_MENU.len();
        match k.code {
            KeyCode::Up => {
                self.cursor = (self.cursor + n - 1) % n;
                MenuOutcome::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1) % n;
                MenuOutcome::None
            }
            KeyCode::Enter => MenuOutcome::Activate(STORY_MENU[self.cursor].command),
            KeyCode::Esc => MenuOutcome::Close,
            _ => {
                let pressed = KeySpec::from_key_event(k);
                match STORY_MENU
                    .iter()
                    .find(|it| first_key(km, it.command).is_some_and(|s| s == pressed))
                {
                    Some(it) => MenuOutcome::Activate(it.command),
                    None => MenuOutcome::None,
                }
            }
        }
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────────

/// Where the menu was drawn: its frame, and one rect per item row.
pub struct MenuRects {
    pub area: Rect,
    pub items: Vec<(usize, Rect)>,
}

/// The key column's text for each item, in menu order (empty where a command
/// carries no binding at all — the guard test forbids that in the defaults, but
/// a user's `[keymap.browser]` can still unbind one).
fn key_labels(km: &KeyMap) -> Vec<String> {
    STORY_MENU
        .iter()
        .map(|it| first_key(km, it.command).map(|s| key_label(&s)).unwrap_or_default())
        .collect()
}

/// The menu's frame for an anchor row, clamped inside `pane`.
///
/// Below the row when there is room, flipped above it when there is not, and
/// pulled back inside the pane on both axes either way — a popup that spills off
/// the edge is a popup that cannot be read or clicked.
pub fn menu_rect(km: &KeyMap, anchor: Rect, pane: Rect) -> Rect {
    let label_w = STORY_MENU.iter().map(|it| UnicodeWidthStr::width(it.label)).max().unwrap_or(0);
    let key_w = key_labels(km).iter().map(|s| UnicodeWidthStr::width(s.as_str())).max().unwrap_or(0);
    // ` label  key ` inside two border columns.
    let w = (label_w + key_w + 6) as u16;
    let h = STORY_MENU.len() as u16 + 2;
    let w = w.min(pane.width);
    let h = h.min(pane.height);

    let below = anchor.y.saturating_add(1);
    let y = if below.saturating_add(h) <= pane.bottom() {
        below
    } else {
        // Flip above the row; if there is no room there either the clamp below
        // puts it wherever it does fit.
        anchor.y.saturating_sub(h)
    };
    let max_x = pane.right().saturating_sub(w);
    let max_y = pane.bottom().saturating_sub(h);
    let x = anchor.x.clamp(pane.x, max_x.max(pane.x));
    let y = y.clamp(pane.y, max_y.max(pane.y));
    Rect::new(x, y, w, h)
}

// ── Draw ──────────────────────────────────────────────────────────────────────

/// Draw the menu over the list, anchored beside `anchor` and clamped to `pane`.
pub fn draw_story_menu(
    menu: &StoryMenu,
    anchor: Rect,
    pane: Rect,
    km: &KeyMap,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> MenuRects {
    let area = menu_rect(km, anchor, pane);
    let item_style = cs.theme.get("dialog.story_menu.item").style;

    // The shared panel chrome rather than `draw_dialog`: a context menu carries
    // no title, and `draw_dialog` always draws a title strip — an empty one
    // leaves `┤  ├` notched into the top border, which reads as a dialog that
    // forgot its name rather than as a popup that never had one.
    //
    // Fill first, and with `Style::reset()`: the list underneath is painted, and
    // the highlighted row under an open menu carries REVERSED. Anything less
    // than a reset leaves that bleeding through (the same opacity `draw_dialog`
    // gives every other modal).
    let fill = Style::reset().patch(item_style);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(fill);
            }
        }
    }
    let border = cs.theme.get("dialog.story_menu.border");
    let box_style = match border.border.unwrap_or(BorderStyle::None) {
        // A menu without a frame is a menu you cannot tell from the list.
        BorderStyle::None => BorderStyle::Single,
        other => other,
    };
    let frame = draw_panel(
        buf,
        &PanelSpec {
            area,
            border_selector: "dialog.story_menu.border",
            border_color: Some(border.style),
            border_style: Some(box_style),
            glyphs: &cs.dialog_glyphs,
            header_on: false,
            strip: None,
            body_fill: Some(fill),
        },
        &cs.theme,
    );
    let content = frame.content;
    let sel_style = cs.theme.get("dialog.story_menu.item:selected").style;
    let key_style = cs.theme.get("dialog.story_menu.key").style;

    let keys = key_labels(km);
    let key_w = keys.iter().map(|s| UnicodeWidthStr::width(s.as_str())).max().unwrap_or(0) as u16;

    let mut items = Vec::new();
    for (i, it) in STORY_MENU.iter().enumerate() {
        let y = content.y + i as u16;
        if y >= content.bottom() {
            break;
        }
        let row = Rect::new(content.x, y, content.width, 1);
        let selected = i == menu.cursor;
        let base = if selected { sel_style } else { item_style };
        // Paint the whole row first: the highlight is a band across the item AND
        // its key, not a coloured label with dim text beside it.
        for x in row.x..row.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(Style::reset().patch(base));
            }
        }
        draw_str_clipped(buf, row.x + 1, y, it.label, base, row);
        let key = &keys[i];
        if !key.is_empty() {
            let kx = row.right().saturating_sub(1 + key_w).max(row.x);
            let kstyle = if selected { base } else { key_style };
            draw_str_clipped(buf, kx, y, key, kstyle, row);
        }
        items.push((i, row));
    }

    MenuRects { area, items }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

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
        assert_eq!(key_labels(&km()), vec!["Enter", "o", "f", "Shift+H", "u"]);
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
