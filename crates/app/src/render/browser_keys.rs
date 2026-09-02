//! The story browser's key reference — the dialog behind `?` (SQ-1227).
//!
//! Its own dialog rather than the game's hotkey panel: `draw_hotkey_dialog`
//! renders `AppState::hotkeys`, which is a hand-authored layout of IN-GAME
//! commands and reaches the browser's bindings not at all — and the browser runs
//! before there is an `AppState` to read it from. So this one is built straight
//! from the two things the picker does have, the resolved [`KeyMap`] and the
//! command registry: one row per `Context::Browser` command, every key that
//! reaches it, and the registry's own description.
//!
//! That means it cannot drift. A command added to the registry and bound in
//! `keymap.rs` appears here the same day, and one that is rebound in
//! `[keymap.browser]` shows the user's key — there is no authored table for
//! either to disagree with.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::browser::key_label;
use crate::colors::ColorScheme;
use crate::keymap::{Context, KeyMap};
use crate::render::dialog::{
    draw_dialog, ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement,
};
use crate::render::draw_str_clipped;

/// One reference row: every key that reaches a command, and what it does.
///
/// Grouped by command NAME, not by the full command-string: the four
/// `move-selection` bindings are one idea with eight keys, and eight rows saying
/// the same sentence is a reference nobody finishes reading.
pub fn key_rows(km: &KeyMap) -> Vec<(String, String)> {
    let mut order: Vec<&str> = Vec::new();
    let mut keys: Vec<Vec<String>> = Vec::new();
    for (spec, cmd) in km.for_context(Context::Browser) {
        let name = cmd.split_whitespace().next().unwrap_or("");
        let idx = match order.iter().position(|n| *n == name) {
            Some(i) => i,
            None => {
                order.push(name);
                keys.push(Vec::new());
                order.len() - 1
            }
        };
        let label = key_label(spec);
        if !keys[idx].contains(&label) {
            keys[idx].push(label);
        }
    }
    order
        .into_iter()
        .zip(keys)
        .map(|(name, ks)| {
            let desc = crate::slash::find_command(name)
                .map(|c| c.description.to_string())
                .unwrap_or_else(|| name.to_string());
            (ks.join("/"), desc)
        })
        .collect()
}

/// Draw the key reference over `area`. `None` when the pane is too small to hold
/// a dialog at all.
pub fn draw_browser_keys(
    km: &KeyMap,
    area: Rect,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> Option<DialogRects> {
    if area.height < 6 || area.width < 30 {
        return None;
    }
    let rows = key_rows(km);
    let key_w = rows.iter().map(|(k, _)| UnicodeWidthStr::width(k.as_str())).max().unwrap_or(0);
    let widest = rows
        .iter()
        .map(|(k, d)| UnicodeWidthStr::width(k.as_str()).max(key_w) + 2 + UnicodeWidthStr::width(d.as_str()))
        .max()
        .unwrap_or(0);
    // +2 borders, +2 so neither column touches the frame.
    let w = ((widest + 4) as u16).min(area.width);
    // +3: two borders plus the button row `draw_dialog` carves out of the
    // content — the arithmetic SQ-0599 had to fix in the hotkey panel, and the
    // reason the last row here is visible at every size that can hold it.
    let h = ((rows.len() as u16).saturating_add(3)).min(area.height);

    let st = DialogStyle::from_colors(cs);
    let buttons = &[DialogButton { id: ButtonId::Done, label: "Done" }];
    let spec = DialogSpec {
        title: "Story browser keys (Esc: close)",
        placement: Placement::Centered { w, h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    let key_style = cs.theme.get("hotkey_key").style;
    let desc_style = cs.theme.get("dialog.background").style;
    for (y, (k, d)) in (content.y..).zip(rows.iter()) {
        if y >= content.bottom() {
            break;
        }
        draw_str_clipped(buf, content.x + 1, y, k, key_style, content);
        let dx = content.x + 1 + key_w as u16 + 2;
        if dx < content.right() {
            draw_str_clipped(buf, dx, y, d, desc_style, content);
        }
    }
    Some(rects)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_lists_every_browser_command_once() {
        let km = KeyMap::default();
        let rows = key_rows(&km);
        // One row per command, and the eight movement keys are one of them.
        let move_row = rows.iter().find(|(k, _)| k.contains('↑')).expect("a movement row");
        assert!(move_row.0.contains('k') && move_row.0.contains('↓'), "{move_row:?}");
        // The new gestures are in it, spelled the way the footer spells them.
        assert!(rows.iter().any(|(k, d)| k == "Space" && d.contains("per-story menu")), "{rows:?}");
        assert!(rows.iter().any(|(k, _)| k == "?"), "{rows:?}");
        assert!(rows.iter().any(|(k, _)| k == "Tab/i"), "Tab leads the info row: {rows:?}");
        assert!(rows.iter().any(|(k, _)| k == "o/Shift+Enter"), "o leads launch options: {rows:?}");
    }

    #[test]
    fn the_dialog_draws_its_rows_and_refuses_a_tiny_pane() {
        let cs = ColorScheme::terminal_default();
        let km = KeyMap::default();
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        let rects = draw_browser_keys(&km, area, &cs, &mut buf).expect("draws");
        let text: String = (rects.area.y..rects.area.bottom())
            .map(|y| {
                (rects.area.x..rects.area.right())
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Story browser keys"), "{text}");
        assert!(text.contains("launch the selected story"), "{text}");
        assert!(text.contains("Done"), "{text}");

        let tiny = Rect::new(0, 0, 20, 4);
        let mut small = Buffer::empty(tiny);
        assert!(draw_browser_keys(&km, tiny, &cs, &mut small).is_none());
    }
}
