//! A reusable popup context-menu widget: bordered chrome, an optional key
//! column, click hit-rects and Up/Down/Enter/Esc key handling — factored out
//! of `story_menu` (SQ-1227) so a second menu (the room panel's right-click
//! menu, SQ-1265) can share the same chrome and key handling instead of
//! re-implementing it.
//!
//! This module owns none of the bookkeeping about WHAT the menu was opened
//! for (a story's list index, a room id, …) or WHICH `KeyMap` context its
//! items' keys live in — those stay with each caller (`story_menu::StoryMenu`
//! reads `Context::Browser`, `room_menu::RoomMenu` reads `Context::Map`), so
//! this stays a pure function of an item list and a cursor.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::colors::ColorScheme;
use crate::keymap::{KeyMap, KeySpec};
use crate::render::draw_str_clipped;
use crate::render::paneframe::BorderStyle;
use crate::render::panel::{draw_panel, PanelSpec};

/// One menu row: the registry command it runs, and what it is called here.
pub struct MenuItem {
    /// A full command-string, dispatched exactly as a key bound to it would be
    /// (in whatever `Context` the caller's items live in).
    pub command: &'static str,
    /// The row's text. A trailing `…` is the usual promise that the item opens
    /// something to answer rather than acting straight away.
    pub label: &'static str,
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

/// Where the menu was drawn: its frame, and one rect per item row.
pub struct MenuRects {
    pub area: Rect,
    pub items: Vec<(usize, Rect)>,
}

/// Route a keystroke against `items`/`*cursor`.
///
/// Up/Down wrap, Enter activates, Esc closes — and an item's OWN hotkey
/// (resolved by `key_of`, in whatever `KeyMap` context the caller reads)
/// activates it directly, so the menu never gets in the way of somebody who
/// already knows the key it is teaching them.
pub fn on_key(
    cursor: &mut usize,
    items: &'static [MenuItem],
    k: KeyEvent,
    km: &KeyMap,
    key_of: impl Fn(&KeyMap, &str) -> Option<KeySpec>,
) -> MenuOutcome {
    let n = items.len();
    match k.code {
        KeyCode::Up => {
            *cursor = (*cursor + n - 1) % n;
            MenuOutcome::None
        }
        KeyCode::Down => {
            *cursor = (*cursor + 1) % n;
            MenuOutcome::None
        }
        KeyCode::Enter => MenuOutcome::Activate(items[*cursor].command),
        KeyCode::Esc => MenuOutcome::Close,
        _ => {
            let pressed = KeySpec::from_key_event(k);
            match items.iter().find(|it| key_of(km, it.command).is_some_and(|s| s == pressed)) {
                Some(it) => MenuOutcome::Activate(it.command),
                None => MenuOutcome::None,
            }
        }
    }
}

/// The key column's text for each item, in menu order (empty where a command
/// carries no binding at all in the caller's context).
pub(crate) fn key_labels(
    items: &[MenuItem],
    km: &KeyMap,
    key_of: &impl Fn(&KeyMap, &str) -> Option<KeySpec>,
) -> Vec<String> {
    items
        .iter()
        .map(|it| key_of(km, it.command).map(|s| crate::browser::key_label(&s)).unwrap_or_default())
        .collect()
}

/// The menu's frame for an anchor rect, clamped inside `pane`.
///
/// Below the anchor when there is room, flipped above it when there is not,
/// and pulled back inside the pane on both axes either way — a popup that
/// spills off the edge is a popup that cannot be read or clicked.
pub fn menu_rect(
    items: &[MenuItem],
    km: &KeyMap,
    key_of: impl Fn(&KeyMap, &str) -> Option<KeySpec>,
    anchor: Rect,
    pane: Rect,
) -> Rect {
    let label_w = items.iter().map(|it| UnicodeWidthStr::width(it.label)).max().unwrap_or(0);
    let key_w =
        key_labels(items, km, &key_of).iter().map(|s| UnicodeWidthStr::width(s.as_str())).max().unwrap_or(0);
    // ` label  key ` inside two border columns.
    let w = (label_w + key_w + 6) as u16;
    let h = items.len() as u16 + 2;
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

/// Draw the menu over whatever is behind it, anchored beside `anchor` and
/// clamped to `pane`.
///
/// Reuses the story menu's chrome selectors (`dialog.story_menu.*`) rather
/// than inventing new ones for the second caller — one popup-menu look,
/// themed once (SQ-1265).
#[allow(clippy::too_many_arguments)]
pub fn draw_menu(
    items: &'static [MenuItem],
    cursor: usize,
    km: &KeyMap,
    key_of: impl Fn(&KeyMap, &str) -> Option<KeySpec>,
    anchor: Rect,
    pane: Rect,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> MenuRects {
    let area = menu_rect(items, km, &key_of, anchor, pane);
    let item_style = cs.theme.get("dialog.story_menu.item").style;

    // The shared panel chrome rather than `draw_dialog`: a context menu carries
    // no title, and `draw_dialog` always draws a title strip — an empty one
    // leaves `┤  ├` notched into the top border, which reads as a dialog that
    // forgot its name rather than as a popup that never had one.
    //
    // Fill first, and with `Style::reset()`: whatever is underneath is painted
    // already, and a highlighted row under an open menu carries REVERSED.
    // Anything less than a reset leaves that bleeding through.
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
        // A menu without a frame is a menu you cannot tell from what is behind it.
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

    let keys = key_labels(items, km, &key_of);
    let key_w = keys.iter().map(|s| UnicodeWidthStr::width(s.as_str())).max().unwrap_or(0) as u16;

    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let y = content.y + i as u16;
        if y >= content.bottom() {
            break;
        }
        let row = Rect::new(content.x, y, content.width, 1);
        let selected = i == cursor;
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
        out.push((i, row));
    }

    MenuRects { area, items: out }
}
