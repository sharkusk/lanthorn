//! Hotkey dialog overlay — draw_hotkey_dialog.
//!
//! Renders a centered bordered box over the terminal area listing the hotkey
//! groups from state.hotkeys. Each group has a title and a list of
//! "KEY  label" rows. A footer shows how to close the dialog.
//! Only drawn when state.overlays.hotkey_dialog == true.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::render::{draw_str_clipped, put_str};
use crate::state::AppState;

// ── draw_hotkey_dialog ────────────────────────────────────────────────────────

/// Render the hotkey dialog overlay onto `buf` using the full terminal `area`.
///
/// The overlay is a centered bordered panel rendered via draw_dialog (opaque
/// background fixes command-panel bleed, #17). Groups from state.hotkeys.groups
/// are listed with their title and "KEY  label" rows.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_hotkey_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    if area.height < 6 || area.width < 30 {
        return None;
    }

    // Build rows to determine height.
    let rows = build_rows(state);

    // Target: at most 60 wide; tall enough for rows + chrome overhead.
    let panel_w = area.width.min(60);
    // +3: border top/bottom, plus the button row `draw_dialog` carves out of the
    // content area. Asking for only +2 left the content one row short of `rows`,
    // so the panel silently dropped its last entry at EVERY terminal size — the
    // final group's last command simply could not be seen.
    let panel_h = ((rows.len() as u16).saturating_add(3)).min(area.height);
    if panel_w < 20 || panel_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let prefix_label = state.hotkeys.prefix.label();
    // Advertise the command-palette transition ('/' promotes this dialog into the
    // fuzzy command palette — SQ-0419) alongside the close hint.
    let title = format!("Commands ({prefix_label}: close  /: palette)");

    let spec = DialogSpec {
        title: title.as_str(),
        placement: Placement::Centered { w: panel_w, h: panel_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // ── Render rows ───────────────────────────────────────────────────────────

    let heading_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .patch(state.colors.theme.get("dialog.title").style);
    let key_style = state.colors.theme.get("hotkey_key").style;
    let label_style = state.colors.theme.get("dialog.background").style;

    for (y, row) in (content.y..).zip(rows.iter()) {
        if y >= content.bottom() {
            break;
        }
        if let Some(stripped) = row.strip_prefix("##") {
            draw_str_clipped(buf, content.x, y, stripped.trim(), heading_style, content);
        } else if row == "---" {
            // blank separator — skip
        } else if let Some((key_part, label_part)) = row.split_once("  ") {
            let kw = key_part.len() as u16;
            put_str(buf, content.x as i32, y as i32, key_part, key_style, content);
            let label_x = content.x + kw + 2;
            if label_x < content.right() {
                // Ellipsize rather than let a long label run into the border.
                // The authored defaults are written to fit; a user-configured
                // entry falls back to a registry description, which is a full
                // sentence and routinely does not.
                let avail = content.right().saturating_sub(label_x) as usize;
                let shown = ellipsize(label_part, avail);
                draw_str_clipped(buf, label_x, y, &shown, label_style, content);
            }
        } else {
            draw_str_clipped(buf, content.x, y, row, label_style, content);
        }
    }

    Some(rects)
}

/// Shorten `s` to at most `width` cells, ending in `…` when it had to be cut.
fn ellipsize(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        return s.to_string();
    }
    match width {
        0 => String::new(),
        1 => "…".to_string(),
        _ => chars[..width - 1].iter().collect::<String>() + "…",
    }
}

/// Build text rows for the dialog panel.
/// Section headings are prefixed with "##"; separators are "---".
///
/// No blank separator rows: one used to trail every group, costing a panel row
/// each, and the panel silently drops any row past its bottom edge — so those
/// blanks came straight out of the last group's visibility on a short terminal.
/// Reclaiming them is what paid for the "Map" group (SQ-0599) without making
/// the panel taller than it already was; the styled group headings separate the
/// sections on their own.
fn build_rows(state: &AppState) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();

    for (title, cmds) in state.hotkeys.groups.iter() {
        rows.push(format!("## {title}"));
        for (letter, cmd_str, authored) in cmds {
            let key_label = letter.to_string();
            let (first, args) = match cmd_str.split_once(char::is_whitespace) {
                Some((c, a)) => (c, a.trim()),
                None => (cmd_str.as_str(), ""),
            };
            let desc = crate::slash::find_command(first)
                .map(|c| c.description)
                .unwrap_or(cmd_str.as_str());
            // An authored label wins: a registry description documents every
            // argument form the *slash* command takes ("zoom the map in/out,
            // reset, or step by signed n"), and a panel entry runs one fixed
            // command with no way to pass an argument at all — so that text is
            // both identical across sibling entries and untrue of each.
            //
            // Without one, an entry that carries arguments leads with them, so
            // siblings at least differ; rows are clipped to the panel width, so
            // the distinguishing word has to come first to survive the clip.
            let label = match authored {
                Some(l) => l.clone(),
                None if args.is_empty() => desc.to_string(),
                None => format!("{args} — {desc}"),
            };
            rows.push(format!("{:<3} {}", key_label, label));
        }
    }

    rows
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};

    use super::draw_hotkey_dialog;
    use crate::render::dialog::ButtonId;
    use crate::state::AppState;

    fn buf_text(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right() {
                if let Some(cell) = buf.cell((x, y)) {
                    s.push_str(cell.symbol());
                }
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn draw_hotkey_dialog_shows_group_title_and_command() {
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);
        // Session leads the panel, and the map groups name themselves as such.
        assert!(text.contains("Session"), "expected the 'Session' group heading in dialog");
        assert!(text.contains("global settings"), "expected Session's 'global settings' label in dialog");
        assert!(
            text.contains("Map \u{b7} Layers"),
            "expected the 'Map \u{b7} Layers' heading: the renderer draws flat headings, so a \
             sub-section can only be spelled in the title"
        );
    }

    #[test]
    fn draw_hotkey_dialog_shows_authored_letter() {
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let rects = draw_hotkey_dialog(&state, area, &mut buf).expect("dialog should draw");
        let content = rects.content;

        // The rename-room row is authored with leader letter 'r'; find its row by
        // locating the row whose content-area text carries that authored panel
        // label, then check the row's key column is 'r'. (This used tidy-map until
        // the Layout group was removed from the panel.)
        let tidy_desc = "rename room";
        let mut found_tidy_row = false;
        for y in content.y..content.bottom() {
            let mut line = String::new();
            for x in content.x..content.right() {
                if let Some(cell) = buf.cell((x, y)) {
                    line.push_str(cell.symbol());
                }
            }
            if line.contains(tidy_desc) {
                found_tidy_row = true;
                let key_cell = buf.cell((content.x, y)).expect("key cell");
                assert_eq!(key_cell.symbol(), "r", "expected authored letter 'r' in the key column of the rename row");
            }
        }
        assert!(found_tidy_row, "expected a row containing rename-room's panel label in the dialog content");

        // The old global-keymap chord label must not appear anywhere.
        let text = buf_text(&buf);
        assert!(!text.contains("^T"), "old chord label '^T' should not appear in the dialog");

        // The Layers group's cycle-layer entry shows its authored panel label,
        // not the raw "cycle-layer next" command string.
        assert!(text.contains("next map layer"), "expected cycle-layer's panel label in dialog");
        assert!(!text.contains("cycle-layer next"), "raw command string should not be shown");
    }

    /// SQ-0599: every default entry's label has to fit the panel. They used to
    /// be full registry sentences, which ran past the border and were cut
    /// mid-word — and nothing showed that text had been lost.
    #[test]
    fn every_default_panel_label_fits_without_ellipsis() {
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 100, 44);
        let mut buf = Buffer::empty(area);
        let rects = draw_hotkey_dialog(&state, area, &mut buf).expect("dialog draws");
        let content = rects.content;

        let mut rows = 0;
        for y in content.y..content.bottom() {
            let line: String = (content.x..content.right())
                .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                .collect();
            if line.trim().is_empty() {
                continue;
            }
            rows += 1;
            assert!(!line.contains('…'), "label clipped at the panel edge: {line:?}");
        }
        assert!(rows > 10, "the panel actually rendered its groups ({rows} rows)");

        // Every group survives — nothing is pushed off the bottom at this size.
        let text = buf_text(&buf);
        for (title, _) in &state.hotkeys.groups {
            assert!(text.contains(title.as_str()), "group {title:?} missing from the panel");
        }
    }

    /// The panel sized itself for its rows plus the two borders but not the
    /// button row `draw_dialog` carves out, so the very last entry was cut at
    /// every terminal size — `reset-game` could not be seen at all.
    #[test]
    fn the_last_entry_of_the_last_group_is_visible() {
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let (title, cmds) = state.hotkeys.groups.last().expect("a last group").clone();
        let (key, _, label) = cmds.last().expect("a last entry").clone();
        let label = label.expect("the default layout authors every label");

        let area = Rect::new(0, 0, 100, 44);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf).expect("dialog draws");
        let text = buf_text(&buf);
        assert!(text.contains(&label), "last entry of {title:?} ('{key}' → {label:?}) was cut off");
    }

    #[test]
    fn a_label_too_long_for_the_panel_is_ellipsized() {
        assert_eq!(super::ellipsize("short", 20), "short");
        assert_eq!(super::ellipsize("abcdefghij", 5), "abcd…");
        assert_eq!(super::ellipsize("abc", 1), "…");
        assert_eq!(super::ellipsize("abc", 0), "");
    }

    #[test]
    fn draw_hotkey_dialog_shows_close_hint() {
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);
        // Footer or title should show close hint
        assert!(text.contains("close") || text.contains("q"), "expected close hint");
    }

    #[test]
    fn draw_hotkey_dialog_advertises_palette_transition() {
        // The '/' → command palette hint must be discoverable in the dialog (SQ-0419).
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);
        assert!(text.contains("palette"), "expected a '/: palette' hint in the hotkey dialog");
    }

    #[test]
    fn draw_hotkey_dialog_shows_dialog_chrome() {
        // Render test: dialog shows [X] close button and [Done] button.
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let rects_opt = draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);

        assert!(text.contains('✕'), "[X] close button should be visible");
        assert!(text.contains("Done"), "[Done] button should be visible");

        let rects = rects_opt.expect("draw_hotkey_dialog should return DialogRects");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1);
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }

    #[test]
    fn draw_hotkey_dialog_bg_opaque_over_map_cell() {
        // Render test: the hotkey dialog background is OPAQUE over a pre-filled
        // map cell (no bleed — the underlying cell's bg/REVERSED is replaced).
        // This verifies fix for issue #17 (command-panel current-room color bleed).
        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;

        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);

        // Pre-fill a REVERSED cell with Red bg in the center where the dialog will sit.
        // The dialog is centered at ~col 10..70, row 0..40 (panel_w=60).
        // Place the sentinel cell near center.
        let sentinel_col = 40u16;
        let sentinel_row = 20u16;
        if let Some(cell) = buf.cell_mut((sentinel_col, sentinel_row)) {
            cell.set_symbol("M")
                .set_style(ratatui::style::Style::new()
                    .bg(Color::Red)
                    .add_modifier(Modifier::REVERSED));
        }

        draw_hotkey_dialog(&state, area, &mut buf);

        // After drawing, the dialog's opaque fill should have replaced the cell.
        let cell = buf.cell((sentinel_col, sentinel_row)).unwrap();
        assert!(
            !cell.modifier.contains(Modifier::REVERSED),
            "dialog opaque bg must clear REVERSED modifier (no bleed)"
        );
        assert_ne!(
            cell.bg,
            Color::Red,
            "dialog opaque bg must replace the Red map background (no bleed)"
        );
        // And [X] should appear somewhere in the buffer.
        let text = buf_text(&buf);
        assert!(text.contains('✕'), "[X] must be present in the dialog");
    }
}
