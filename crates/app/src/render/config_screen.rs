//! Config-screen modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::config::BackgroundTidy;
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Row definitions: (display name, type tag, one-line description shown as a
/// tooltip under the list). Rows are addressed by index in the edit helpers
/// (`config_toggle_or_edit` / `config_cycle` in `input.rs`) and in
/// `config_row_value` below — APPEND new rows, never reorder.
///
/// **Or insert one and renumber all FOUR matches together**, which is what
/// SQ-0873 did to put `period_look` beside the switch it is narrower than. They
/// are `config_row_value` here, and `config_toggle_or_edit`, `config_cycle` and
/// `one_run_key_for_row` in `input.rs`; miss one and a row silently edits its
/// neighbour. Nothing catches that — `config_row_count_matches_config_rows_len`
/// checks the COUNT, and the count is right either way. Two tests were reaching
/// their row by literal index and started editing the row below it; both now
/// look it up by name, which is the pattern to copy.
/// Every row is a GLOBAL config setting (SQ-1161). Editing one changes a working
/// copy and nothing else; Save writes `config.toml` AND applies the change to the
/// running session; Cancel discards. There are no action rows — a thing that DOES
/// something rather than holding a value cannot be written to the global config
/// and is not part of Save, so it belongs on a slash command, not here.
///
/// A handful of settings genuinely cannot be applied to a session already under
/// way, because they are consumed once when the story boots (`user_dir` resolves
/// the save/map folders, `undo_levels` sets the machine's cap, `interpreter_number`
/// writes header byte $1E). Those rows SAY "on next launch" in their own
/// description — a row that appears to work and does not is worse than an honest
/// one.
pub(crate) const CONFIG_ROWS: &[(&str, ConfigRowKind, &str)] = &[
    ("user_dir",             ConfigRowKind::Path, "Where lanthorn keeps saves, maps, and settings (default ~/.lanthorn). Takes effect on next launch: the story in front of you keeps the folders it was launched with."),
    ("auto_load",            ConfigRowKind::Bool, "Resume your last session automatically on launch."),
    ("auto_save",            ConfigRowKind::Bool, "Save the archive after every turn, not just on quit or Ctrl+S."),
    ("prompt_save_on_quit",  ConfigRowKind::Bool, "Ask to save when you quit (only matters when auto_save is off)."),
    ("prompt_load_on_launch",ConfigRowKind::Bool, "Offer to resume a found save on launch (only when auto_load is off)."),
    ("show_room_numbers",    ConfigRowKind::Bool, "Print each room's internal #id inside its map box."),
    ("background_tidy",      ConfigRowKind::Enum, "When to re-tidy the map as rooms appear: off / every_room / on_overlap / debounced."),
    ("aux_storage",          ConfigRowKind::Enum, "Where Z-machine v5 auxiliary save data lives: ask / archive / global."),
    ("honor_game_colours",   ConfigRowKind::Bool, "Let games pick their own colours; off = your theme owns every colour."),
    ("period_look",          ConfigRowKind::Bool, "Paint a v1-v4 story the way its machine's own interpreter did — its page, ink, status band and cursor (only off a release disk, and only if you have not themed those yourself)."),
    ("honor_timed_input",    ConfigRowKind::Bool, "Let real-time games run timed-input interrupts (clocks, countdowns, the bomb in Border Zone)."),
    ("enable_sound",         ConfigRowKind::Bool, "Play sound effects and music."),
    ("volume",               ConfigRowKind::Num,  "Master volume, 0-100. Use ← / → to adjust."),
    ("mouse",                ConfigRowKind::Bool, "Capture the mouse for clicks, wheel scrolling, and in-game mouse input."),
    ("command_bar",          ConfigRowKind::Bool, "Type into a pinned bottom command bar instead of the inline > prompt."),
    ("mouse_wheel_invert",   ConfigRowKind::Bool, "Reverse the mouse-wheel scroll direction (for terminals with 'natural' scrolling)."),
    ("show_status_bar",      ConfigRowKind::Bool, "Show the top status bar (location, score, moves, time)."),
    ("watch_style",          ConfigRowKind::Bool, "Live-reload style.toml automatically whenever the file changes on disk."),
    ("record_turn_history",  ConfigRowKind::Bool, "Record a per-turn rewind/replay history — enables Rewind, but grows the archive and holds per-turn blobs in memory."),
    ("undo_levels",          ConfigRowKind::Num,  "How many in-memory undo snapshots to keep (0 disables undo). Use ← / → to adjust. Takes effect on next launch (or after @restart): the running story keeps the cap it booted with."),
    ("interpreter_number",   ConfigRowKind::Num,  "Z-machine interpreter number (header byte 1Eh); changes colour behaviour on some Infocom games (e.g. Beyond Zork). ← / → to adjust. Takes effect on next launch (or after @restart): the header byte is written when the story boots."),
    ("hint_skip_screen_warning", ConfigRowKind::Bool, "Auto-skip the InvisiClues 'your screen is only N characters wide' banner when opening izm hints, landing straight on the topic menu."),
    ("text_margin_x",        ConfigRowKind::Num,  "Blank columns reserved on each side inside the story text pane. Imported from garglk tmarginx. Use ← / → to adjust."),
    ("text_margin_y",        ConfigRowKind::Num,  "Blank rows reserved above and below the story text. Imported from garglk tmarginy. Use ← / → to adjust."),
    ("v6_render",            ConfigRowKind::Enum, "How v6 graphical games (Zork Zero) render: hybrid (crisp terminal story in a scaled pixel frame), raster (whole pane as one pixel image) or extended (raster at a whole magnification, the frame grown downward for more story rows)."),
    ("v6_arrow_keys",        ConfigRowKind::Bool, "Forward arrow keypresses to v6 stories (some bind them to movement); off = arrows drive lanthorn's scrollback and map panning instead."),
    ("v6_pixel_lock",        ConfigRowKind::Bool, "Scale v6 artwork by whole device pixels per art pixel (0.5x/1x/1.5x on a 320-wide rendition, 1x/2x/3x on the Mac mono and EGA ones) instead of stretching it to fill the pane: crisper art and seamless borders, at the cost of a wider margin. No effect under half-blocks, which draws cells rather than device pixels and has no rung to snap to."),
    ("guidance",             ConfigRowKind::Bool, "Lanthorn's Guiding Light: help offered while you play — the words the parser knows, a completed noun, a caution before a move that cannot be undone. Marked in the margin with its own glyph (style.toml's gutter.assist), never in the story's voice."),
    ("guidance_probe",       ConfigRowKind::Bool, "Vet the Guiding Light's word suggestions before showing them: each candidate is tried in a silent throwaway copy of the game and only what actually did something is offered. Nothing it does reaches the screen, your saves, or the game you are playing. Off, the light still offers — it just names what the dictionary holds instead of recommending."),
    ("hide_adult_words",     ConfigRowKind::Bool, "Keep the strong language out of panels that list a story's whole vocabulary — the command panel's VERB column and its like. Display only: the story still knows every word, typing one works exactly as before, and the Guiding Light still offers it. The words are the `adult_words` line in config.toml, there to be read, shortened or extended."),
    // Appended rather than filed beside `guidance_probe`, where it reads more
    // naturally: three tables in `input.rs` key off a row's INDEX, so inserting
    // in the middle renumbers every row after it in four places at once. The
    // grouping is worth less than not making that edit (SQ-0785).
    ("return_probe",         ConfigRowKind::Bool, "After a move, look for the way BACK in a silent throwaway copy of the game, and put it on the map when it is found — closing the one-way gaps an automap is otherwise full of, without ever assuming that a passage runs both ways. Nothing is recorded unless the copy comes out in the room you left. Off by default; the footprint on the map pane's bottom border switches it for one game."),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigRowKind {
    Path,
    Bool,
    Enum,
    Num,
}

/// Draw the config-screen modal centered over `area`.
/// Does nothing when `state.overlays.config_screen` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_config_screen(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let Some(cs) = &state.overlays.config_screen else { return None };

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    // Chrome overhead + a 2-line tooltip footer for the focused row's description.
    let modal_h = (CONFIG_ROWS.len() as u16 + 8).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // Build DialogStyle from state colors
    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Save,   label: "Save"   },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];

    let spec = DialogSpec {
        title: "Global Settings",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Save),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Draw column headers inside content
    let dialog_bg = state.colors.theme.get("dialog.background").style;
    let hdr_style = dialog_bg.patch(state.colors.theme.get("dialog.list_header").style);
    let name_col_w = 22usize;
    let hdr = format!("{:<width$}  Value", "Setting", width = name_col_w);
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &hdr, hdr_style, content);
    }

    // Row styles
    let normal = state.colors.theme.get("dialog.background").style;
    let selected_style = state.colors.theme.get("dialog.list_selected").style;

    // Reserve the bottom lines for the focused row's tooltip when there's room
    // (header line on top + at least one row + the tooltip footer).
    const TOOLTIP_H: u16 = 2;
    let tooltip_h = if content.height >= 2 + TOOLTIP_H { TOOLTIP_H } else { 0 };

    // Content rows start after the header line and stop above the tooltip footer.
    let rows_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1 - tooltip_h)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    let total = CONFIG_ROWS.len();
    let viewport = rows_area.height as usize;
    *vp_out = viewport;

    // Reserve a 1-column gutter on the right for the scrollbar when overflowing.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, viewport) && content.width >= 2;
    let row_w = if scrollbar_visible { content.width.saturating_sub(1) } else { content.width };
    let row_area = Rect::new(content.x, rows_area.y, row_w, rows_area.height);

    let offset = cs.scroll.display_offset();
    for row in 0..viewport {
        let i = offset + row;
        if i >= total {
            break;
        }
        let (name, _kind, _desc) = CONFIG_ROWS[i];
        let row_y = rows_area.y + row as u16;

        let is_selected = i == cs.scroll.selected;
        let row_style = if is_selected { selected_style } else { normal };

        // Fill row background.
        for col in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(row_style);
            }
        }

        // Build value string.
        let value = config_row_value(&cs.working, i);

        let marker = if is_selected { ">" } else { " " };
        let name_trunc: String = name.chars().take(name_col_w).collect();
        let line = format!("{} {:<width$}  {}", marker, name_trunc, value, width = name_col_w);
        crate::render::draw_str_clipped(buf, row_area.x, row_y, &line, row_style, row_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(rows_area.right().saturating_sub(1), rows_area.y, 1, rows_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            viewport,
            cs.scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }

    // Focused row's description, word-wrapped into the reserved footer, dimmed.
    if tooltip_h > 0 && total > 0 {
        let (_n, _k, desc) = CONFIG_ROWS[cs.scroll.selected.min(total - 1)];
        let tip_y = content.y + content.height - tooltip_h;
        let tip_style = normal.add_modifier(Modifier::DIM);
        for (li, line) in wrap_desc(desc, content.width as usize)
            .into_iter()
            .take(tooltip_h as usize)
            .enumerate()
        {
            crate::render::draw_str_clipped(buf, content.x, tip_y + li as u16, &line, tip_style, content);
        }
    }

    Some(rects)
}

/// Word-wrap `s` to at most `width` display columns per line.
fn wrap_desc(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Build the display value string for row `i` from the working config.
fn config_row_value(cfg: &crate::config::Config, i: usize) -> String {
    match i {
        0 => cfg.user_dir.to_string_lossy().to_string(),
        1 => bool_str(cfg.auto_load),
        2 => bool_str(cfg.auto_save),
        3 => bool_str(cfg.prompt_save_on_quit),
        4 => bool_str(cfg.prompt_load_on_launch),
        5 => bool_str(cfg.show_room_numbers),
        6 => match cfg.background_tidy {
            BackgroundTidy::Off => "off".to_string(),
            BackgroundTidy::EveryRoom => "every_room".to_string(),
            BackgroundTidy::OnOverlap => "on_overlap".to_string(),
            BackgroundTidy::Debounced => "debounced".to_string(),
        },
        7 => match cfg.aux_storage {
            crate::config::AuxStorage::Ask => "ask".to_string(),
            crate::config::AuxStorage::Archive => "archive".to_string(),
            crate::config::AuxStorage::Global => "global".to_string(),
        },
        8 => bool_str(cfg.honor_game_colours),
        9 => bool_str(cfg.period_look),
        10 => bool_str(cfg.honor_timed_input),
        11 => bool_str(cfg.enable_sound),
        12 => cfg.volume.to_string(),
        13 => bool_str(cfg.mouse),
        14 => bool_str(cfg.command_bar),
        15 => bool_str(cfg.mouse_wheel_invert),
        16 => bool_str(cfg.show_status_bar),
        17 => bool_str(cfg.watch_style),
        18 => bool_str(cfg.record_turn_history),
        19 => cfg.undo_levels.to_string(),
        20 => cfg.interpreter_number.map(|n| n.to_string()).unwrap_or_else(|| "default".to_string()),
        21 => bool_str(cfg.hint_skip_screen_warning),
        22 => cfg.text_margin_x.to_string(),
        23 => cfg.text_margin_y.to_string(),
        24 => match cfg.v6_render {
            crate::config::V6RenderMode::Hybrid => "hybrid".to_string(),
            crate::config::V6RenderMode::Raster => "raster".to_string(),
            crate::config::V6RenderMode::Extended => "extended".to_string(),
        },
        25 => bool_str(cfg.v6_arrow_keys),
        26 => bool_str(cfg.v6_pixel_lock),
        27 => bool_str(cfg.guidance),
        28 => bool_str(cfg.guidance_probe),
        29 => bool_str(cfg.hide_adult_words),
        30 => bool_str(cfg.return_probe),
        _ => String::new(),
    }
}

fn bool_str(b: bool) -> String {
    if b { "true".to_string() } else { "false".to_string() }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, ConfigScreenState};

    fn state_with_config_screen() -> AppState {
        let mut s = AppState::default();
        let working = crate::input::clone_config(&s.config);
        s.overlays.config_screen = Some(ConfigScreenState { working, scroll: Default::default() });
        s
    }

    #[test]
    fn draw_config_screen_shows_settings() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_config_screen();
        terminal.draw(|f| {
            draw_config_screen(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("auto_save"), "auto_save row should be visible");
        assert!(content.contains("background_tidy"), "background_tidy row should be visible");
        assert!(content.contains("mouse"), "mouse row should be visible");
    }

    /// SQ-1161: the screen is Global Settings, so every row holds a global config
    /// value — nothing on it merely DOES something. `font_check` was the one
    /// exception (an action row whose answer landed in `style.toml`, outside what
    /// Save writes) and it is gone; the font check is reached by `/run-font-check`,
    /// `--font-check on`, and the first-run offer.
    #[test]
    fn every_row_holds_a_value_and_none_of_them_is_an_action() {
        for (name, kind, desc) in CONFIG_ROWS {
            assert!(
                matches!(kind, ConfigRowKind::Path | ConfigRowKind::Bool | ConfigRowKind::Enum | ConfigRowKind::Num),
                "{name} must hold a value"
            );
            assert!(!desc.is_empty(), "{name} needs a description");
        }
        assert!(
            !CONFIG_ROWS.iter().any(|(n, _, _)| *n == "font_check"),
            "the font check is a command, not a global setting"
        );
    }

    /// SQ-1161: the rows Save cannot apply to a session already under way say so.
    /// Each of these is consumed once when the story boots — the save/map folders,
    /// the machine's undo cap, header byte $1E — so a row that stayed silent would
    /// look like it had worked.
    #[test]
    fn the_boot_only_rows_say_they_land_on_the_next_launch() {
        for name in ["user_dir", "undo_levels", "interpreter_number"] {
            let (_, _, desc) = CONFIG_ROWS
                .iter()
                .find(|(n, _, _)| *n == name)
                .unwrap_or_else(|| panic!("{name} row exists"));
            assert!(
                desc.contains("on next launch"),
                "{name} cannot be applied by Save, so its description must say when it lands: {desc}"
            );
        }
    }

    #[test]
    fn newly_added_rows_toggle_and_adjust_through_their_index() {
        use crate::input::{apply_action, Action};
        use mapper::mapper::Mapper;
        let mut m = Mapper::default();

        // A bool row (record_turn_history) flips via ConfigToggle at its index.
        let mut state = state_with_config_screen();
        let idx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "record_turn_history").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = idx;
        let before = state.overlays.config_screen.as_ref().unwrap().working.record_turn_history;
        apply_action(Action::ConfigToggle, &mut state, &mut m);
        let after = state.overlays.config_screen.as_ref().unwrap().working.record_turn_history;
        assert_ne!(before, after, "record_turn_history row must toggle at its CONFIG_ROWS index");

        // A number row (undo_levels) steps via ConfigCycle at its index.
        let uidx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "undo_levels").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = uidx;
        state.overlays.config_screen.as_mut().unwrap().working.undo_levels = 4;
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.undo_levels, 5);

        // A bool row must flip via BOTH ConfigToggle (Space) and ConfigCycle
        // (←/→) — the two handlers are parallel index matches, so exercise both
        // for the last-added bool row so neither can silently miss it.
        let hidx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "hint_skip_screen_warning").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = hidx;
        let b0 = state.overlays.config_screen.as_ref().unwrap().working.hint_skip_screen_warning;
        apply_action(Action::ConfigToggle, &mut state, &mut m);
        let b1 = state.overlays.config_screen.as_ref().unwrap().working.hint_skip_screen_warning;
        assert_ne!(b0, b1, "hint_skip_screen_warning must toggle via Space");
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        let b2 = state.overlays.config_screen.as_ref().unwrap().working.hint_skip_screen_warning;
        assert_ne!(b1, b2, "hint_skip_screen_warning must flip via ←/→ too");

        // The adult-list switch (SQ-1122) — the row APPENDED after `font_check`,
        // and therefore the one whose index is easiest to add to three of the
        // four matches instead of four. Both handlers, as above.
        let aidx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "hide_adult_words").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = aidx;
        let a0 = state.overlays.config_screen.as_ref().unwrap().working.hide_adult_words;
        assert!(a0, "the switch is on out of the box");
        apply_action(Action::ConfigToggle, &mut state, &mut m);
        let a1 = state.overlays.config_screen.as_ref().unwrap().working.hide_adult_words;
        assert_ne!(a0, a1, "hide_adult_words must toggle via Space");
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        let a2 = state.overlays.config_screen.as_ref().unwrap().working.hide_adult_words;
        assert_ne!(a1, a2, "hide_adult_words must flip via ←/→ too");
        assert_eq!(
            config_row_value(&state.overlays.config_screen.as_ref().unwrap().working, aidx),
            "true",
            "and the value column reports the row it edits, not its neighbour"
        );

        // The text-margin Num rows (SQ-0345) step via ConfigCycle, clamp at 0,
        // and ignore Space (Num rows only respond to ←/→).
        let midx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "text_margin_x").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = midx;
        state.overlays.config_screen.as_mut().unwrap().working.text_margin_x = 1;
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.text_margin_x, 2, "text_margin_x steps up via ←/→");
        apply_action(Action::ConfigToggle, &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.text_margin_x, 2, "Space is a no-op for the Num margin row");
        for _ in 0..3 { apply_action(Action::ConfigCycle(-1), &mut state, &mut m); }
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.text_margin_x, 0, "text_margin_x clamps at 0");

        // The v6_render enum row (SQ-0186) cycles via ConfigCycle at its index,
        // Hybrid -> Raster -> Extended and back (SQ-0895 removed the old third
        // position, SQ-1032 added a different one).
        let vidx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "v6_render").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = vidx;
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.v6_render, crate::config::V6RenderMode::Hybrid);
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.v6_render, crate::config::V6RenderMode::Raster, "v6_render must cycle to Raster");
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.v6_render, crate::config::V6RenderMode::Extended, "v6_render must cycle to Extended");
        apply_action(Action::ConfigCycle(1), &mut state, &mut m);
        assert_eq!(state.overlays.config_screen.as_ref().unwrap().working.v6_render, crate::config::V6RenderMode::Hybrid, "v6_render must cycle back to Hybrid");
    }

    #[test]
    fn focused_row_tooltip_is_rendered() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_config_screen();
        // Focus the record_turn_history row; its description should appear.
        let idx = CONFIG_ROWS.iter().position(|(n, _, _)| *n == "record_turn_history").unwrap();
        state.overlays.config_screen.as_mut().unwrap().scroll.selected = idx;
        terminal.draw(|f| { draw_config_screen(&state, f.area(), f.buffer_mut(), &mut 0); }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(content.contains("rewind"), "the focused row's tooltip text should render");
    }

    #[test]
    fn draw_config_screen_scrollbar_and_paging_on_overflow() {
        use crate::input::{apply_action, Action};
        use mapper::mapper::Mapper;
        // A short terminal so the 10 settings rows can't all fit -> windowed list
        // with a scrollbar, and PageDown advances the selection by ~a viewport.
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_config_screen();
        let mut vp = 0usize;
        terminal.draw(|f| {
            draw_config_screen(&state, f.area(), f.buffer_mut(), &mut vp);
        }).unwrap();
        assert!(vp > 0 && vp < CONFIG_ROWS.len(), "rows should overflow a short modal (vp={vp})");
        // A scrollbar thumb (a background fill, SQ-0782) is drawn when the list overflows.
        let thumb = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme).thumb;
        let has_thumb = terminal.backend().buffer().content().iter().any(|c| c.bg == thumb);
        assert!(has_thumb, "a scrollbar thumb should be drawn when rows overflow");

        // Sync the captured viewport (run loop does this) then PageDown.
        state.modal_list_viewport = vp;
        apply_action(Action::ConfigPage(1), &mut state, &mut Mapper::default());
        let sel = state.overlays.config_screen.as_ref().unwrap().scroll.selected;
        assert!(sel >= vp.saturating_sub(1), "PageDown should advance ~one viewport, got {sel}");
    }

    #[test]
    fn draw_config_screen_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // config_screen = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_config_screen(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_config_screen should be no-op when closed");
    }

    #[test]
    fn draw_config_screen_shows_chrome() {
        // Render test: config screen shows a border + title + [Save]/[Cancel] + [X]
        // with colors from state.colors.dialog_*
        use crate::render::paneframe::BorderStyle;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_config_screen();
        // Use Single border so the title renders on the border row (not overlapping content)
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_config_screen(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        // Title should appear
        // "Global Config", not "Settings": every value on this screen is written to
        // the GLOBAL ~/.lanthorn/config.toml, and calling it settings invited the
        // reading that it applied to the story in front of you.
        assert!(content.contains("Global Settings"), "title 'Global Settings' should be present");
        // Save and Cancel buttons
        assert!(content.contains("Save"), "[Save] button should be visible");
        assert!(content.contains("Cancel"), "[Cancel] button should be visible");
        // Close button (✕)
        assert!(content.contains('✕'), "[X] close button should be visible");

        // DialogRects should be returned
        let rects = rects_out.expect("draw_config_screen should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 2, "should have 2 buttons");

        // Verify button ids
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Save));
        assert!(ids.contains(&ButtonId::Cancel));
    }
}
