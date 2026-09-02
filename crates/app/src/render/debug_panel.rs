//! Debug-inspector renderer (tiled pane in the map slot). Paints the
//! DebugPanelState snapshot: three tabbed windows (left full height; right
//! split top/bottom), each with its tab strip embedded in its top border row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::debug_panel::{self, DebugPanelState, HoverTip, Section};
use crate::engine::DisasmProvenance;
use crate::render::draw_str_clipped;
use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::render::paneframe::{BorderStyle, InsetSegment};
use crate::state::AppState;

/// Redraw the char ranges `clickable_spans` reports within `line` with the
/// UNDERLINED modifier added to `style` (color unchanged), at `x_base + range.start`.
fn underline_clickables(buf: &mut Buffer, x_base: u16, y: u16, line: &str, style: Style, section: Section, area: Rect) {
    for (range, _target) in debug_panel::clickable_spans(section, line) {
        let Some(sub) = line.get(range.clone()) else { continue };
        // `range` is a BYTE range into `line`, but the x position is a display
        // COLUMN — one per char, matching how `draw_str_clipped` walks
        // `line.chars()` (SQ-0638). Converting via the prefix's char count (not
        // its byte length) keeps the underline under the link text even when a
        // multibyte char sits earlier on the line.
        let prefix_chars = line[..range.start].chars().count() as u16;
        let x = x_base + prefix_chars;
        draw_str_clipped(buf, x, y, sub, style.add_modifier(Modifier::UNDERLINED), area);
    }
}

/// Draw the Disassembly section: `disasm_rows` inserts a PC-divider row
/// directly above the instruction at `pc`, so render and the click hit-test
/// (`clickable_at`) always agree on which screen row is which disasm line.
fn draw_disasm(buf: &mut Buffer, content: Rect, panel: &DebugPanelState, state: &AppState) {
    let disasm = &panel.snapshot.disasm;
    let rows = debug_panel::disasm_rows(disasm, panel.pc, content.height as usize);
    let text_rect = Rect::new(content.x + 1, content.y, content.width.saturating_sub(1), content.height);
    // Bound the confidence-tier selectors (SQ-0428) once: each row picks one and
    // reads both its line style and its gutter-mark glyph.
    let theme = &state.colors.theme;
    let pc_style = theme.get("debug.pc").style;
    let executed = theme.get("debug.disasm_executed");
    let rd = theme.get("debug.disasm_rd");
    let soft = theme.get("debug.disasm_soft");
    let data = theme.get("debug.disasm_data");
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        if row_entry.divider {
            let width = content.width.saturating_sub(1) as usize;
            let core = "▼── PC ──▼";
            let text: String = if core.chars().count() >= width {
                core.chars().take(width).collect()
            } else {
                let mut s = core.to_string();
                s.push_str(&"─".repeat(width - core.chars().count()));
                s
            };
            draw_str_clipped(buf, content.x + 1, y, &text, pc_style, content);
            continue;
        }
        let Some(line) = disasm.get(row_entry.line_idx) else { continue };
        // COLOUR and GLYPH are decoupled (SQ-0449): the confidence tier (colour)
        // is driven by CUMULATIVE coverage — a line that EVER ran stays blue —
        // while the `|` gutter marks ONLY the LAST command's execution. Data
        // always wins the colour tier; else ever-executed; else the static tag.
        let addr = line.get(0..6).and_then(|a| u32::from_str_radix(a, 16).ok());
        let ran_ever = addr.is_some_and(|a| panel.snapshot.executed_ever.contains(&a));
        let ran_last = addr.is_some_and(|a| panel.snapshot.executed.contains(&a));
        let prov = panel.snapshot.disasm_prov.get(row_entry.line_idx).copied().unwrap_or(DisasmProvenance::Rd);
        let tier = match prov {
            DisasmProvenance::Data => &data,
            _ if ran_ever => &executed,
            DisasmProvenance::Soft => &soft,
            DisasmProvenance::Rd => &rd,
        };
        let tier_style = tier.style;
        // Gutter: only a line that ran THIS turn gets the executed tier's `|`.
        // A cumulative-executed line is blue (tier == executed, whose glyph is
        // `|`) but must NOT carry the bar unless it ran last turn — so force a
        // space there. Other tiers use their own glyph (default space).
        let mark = if ran_last {
            executed.glyph.as_ref().and_then(|g| g.single.as_deref()).unwrap_or("|")
        } else if ran_ever {
            " " // executed tier chosen for colour, but not last turn → no bar
        } else {
            tier.glyph.as_ref().and_then(|g| g.single.as_deref()).unwrap_or(" ")
        };
        draw_str_clipped(buf, content.x, y, mark, tier_style, content);
        draw_str_clipped(buf, content.x + 1, y, line, tier_style, text_rect);
        underline_clickables(buf, content.x + 1, y, line, tier_style, Section::Disasm, text_rect);
    }
}

/// Draw one window: frame, tab strip, and the active section's content.
/// Returns the strip's per-tab hit-rects (absolute screen coords), so the click
/// handler reads the RENDERED tabs rather than recomputing their geometry.
fn draw_window(buf: &mut Buffer, area: Rect, window: usize, panel: &DebugPanelState, state: &AppState) -> Vec<Rect> {
    if area.width < 2 || area.height < 2 { return Vec::new(); }
    let theme = &state.colors.theme;
    // Active border only on the one truly-focused window: the debug pane must
    // hold focus (not the story pane) AND this be its focused window. The
    // selector drives both the border colour and its style.
    let focused = state.focus == crate::state::Focus::Map && panel.focus == window;
    let border_selector = if focused { "panel.border:active" } else { "panel.border" };
    // Tabs are drawn on the top border row; coerce a None border (missing OR an
    // explicit `style = "none"`) to Single so the strip always has a border row to
    // sit in (preserves the old dialog_box_style None→Single coercion intent).
    let border_style = Some(match theme.get(border_selector).border {
        None | Some(BorderStyle::None) => BorderStyle::Single,
        Some(s) => s,
    });

    // Tab strip: one bracketed segment per section, active = the window's tab.
    // Read the panel's LIVE layout + labels (not the const), so an engine that
    // hides/relabels sections (Scott) renders its own tabs.
    let sections = &panel.tabs[window];
    let segs: Vec<InsetSegment> = sections.iter().enumerate()
        .map(|(i, s)| InsetSegment { text: panel.tab_label(*s), active: i == panel.tab[window] })
        .collect();
    let frame = draw_panel(buf, &PanelSpec {
        area,
        border_selector,
        border_color: None,
        border_style,
        glyphs: &state.colors.dialog_glyphs,
        header_on: true,
        strip: Some(PanelStrip {
            segments: &segs,
            base: theme.get("panel.tab").style,
            active: theme.get("panel.tab:active").style,
        }),
        body_fill: None,
    }, theme);

    // Active section content, clipped to the frame's content rect.
    let section = panel.active_section(window);
    let lines = panel.snapshot.section(section);
    let content = frame.content;
    // Body text is the plain text role (foreground only), so the pane's own
    // background shows through instead of a chrome-black block behind every glyph.
    let body = theme.get("text").style;

    if section == Section::Disasm {
        draw_disasm(buf, content, panel, state);
        return frame.tab_rects;
    }

    if section == Section::Objects {
        draw_objects(buf, content, window, panel, body);
        return frame.tab_rects;
    }

    if section == Section::CallStack {
        draw_callstack(buf, content, window, panel, body);
        return frame.tab_rects;
    }

    if section == Section::Memory {
        draw_memory(buf, content, panel, state, body);
        return frame.tab_rects;
    }

    // List sections apply their per-window scroll offset.
    let scroll = panel.scroll[window];
    for (row, line) in lines.iter().skip(scroll).take(content.height as usize).enumerate() {
        let y = content.y + row as u16;
        draw_str_clipped(buf, content.x, y, line, body, content);
        // Underline any clickable spans (Dict entry-address links); a no-op for
        // the sections `clickable_spans` classifies as inert.
        underline_clickables(buf, content.x, y, line, body, section, content);
    }
    frame.tab_rects
}

/// Draw the Objects section: `objects_rows` interleaves each tree line with
/// its expanded detail lines (if any), so render and the click hit-test
/// (`objects_click_at`) always agree on which screen row is which object row.
/// Tree rows that carry an object id get a ▶/▼ disclosure triangle (clickable
/// to toggle); detail rows are drawn plain and indented.
fn draw_objects(buf: &mut Buffer, content: Rect, window: usize, panel: &DebugPanelState, body: Style) {
    let rows = debug_panel::objects_rows(
        &panel.snapshot.objects, &panel.expanded_objects, &panel.snapshot.object_details,
        panel.scroll[window], content.height as usize,
    );
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        match row_entry {
            debug_panel::ObjRow::Tree { line_idx, obj } => {
                let Some(line) = panel.snapshot.objects.get(*line_idx) else { continue };
                // Disclosure triangle marks a toggleable object row: ▼ when
                // expanded, ▶ when collapsed; a marker-less row (no id) keeps a
                // two-space pad so columns stay aligned.
                let marker = match obj {
                    Some(n) if panel.expanded_objects.contains(n) => "▼ ",
                    Some(_) => "▶ ",
                    None => "  ",
                };
                let text = format!("{marker}{line}");
                draw_str_clipped(buf, content.x, y, &text, body, content);
                // The entry-address `@0x……` link underlines like a disasm ref
                // (the 2-col marker offsets the line text).
                underline_clickables(buf, content.x + 2, y, line, body, Section::Objects, content);
            }
            debug_panel::ObjRow::Detail { obj, di } => {
                let Some(det) = panel.snapshot.object_details.get(obj) else { continue };
                let Some(line) = det.get(*di) else { continue };
                let indented = format!("    {line}");
                draw_str_clipped(buf, content.x, y, &indented, body, content);
                // The `entry @0x……` line underlines like any other address link
                // (SQ-0975); the 4-col indent offsets the line text.
                underline_clickables(buf, content.x + 4, y, line, body, Section::Objects, content);
            }
        }
    }
}

/// Draw the Call Stack section: `stack_rows` interleaves each frame line with
/// its expanded locals detail lines (if any), so render and the two hit-tests
/// (`clickable_at`, `stack_click_at`) always agree on which screen row is which
/// frame line. Frame rows get a ▶/▼ disclosure triangle (a 2-column marker, so
/// the frame text — with its clickable `fn@` address — draws at `content.x + 2`);
/// detail rows are drawn plain and indented.
fn draw_callstack(buf: &mut Buffer, content: Rect, window: usize, panel: &DebugPanelState, body: Style) {
    let rows = debug_panel::stack_rows(
        &panel.snapshot.stack, &panel.expanded_frames, &panel.snapshot.frame_details,
        panel.scroll[window], content.height as usize,
    );
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        match row_entry {
            debug_panel::StackRow::Frame { line_idx, frame } => {
                let Some(line) = panel.snapshot.stack.get(*line_idx) else { continue };
                let marker = match frame {
                    Some(n) if panel.expanded_frames.contains(n) => "▼ ",
                    Some(_) => "▶ ",
                    None => "  ", // e.g. the "(no frames)" line
                };
                draw_str_clipped(buf, content.x, y, marker, body, content);
                draw_str_clipped(buf, content.x + 2, y, line, body, content);
                underline_clickables(buf, content.x + 2, y, line, body, Section::CallStack, content);
            }
            debug_panel::StackRow::Detail { frame, di } => {
                let Some(det) = panel.snapshot.frame_details.get(frame) else { continue };
                let Some(line) = det.get(*di) else { continue };
                let indented = format!("    {line}");
                draw_str_clipped(buf, content.x, y, &indented, body, content);
            }
        }
    }
}

/// Draw `text`, whose first character belongs at logical column `col` of a row
/// scrolled left by `hscroll`, clipped into `content`. Chars, not bytes — the
/// hex dump's character column carries the story's own ZSCII, which reaches
/// well past ASCII (`ä`, `»`) through the §3.8.5 translation table.
fn draw_hscrolled(
    buf: &mut Buffer, content: Rect, y: u16, col: usize, hscroll: usize, text: &str, style: Style,
) {
    let skip = hscroll.saturating_sub(col);
    // Byte offset of the first still-visible char; `None` = the scroll has run
    // past the whole string, so there is nothing on this row to draw.
    let Some((byte, _)) = text.char_indices().nth(skip) else { return };
    let x = content.x.saturating_add(col.saturating_sub(hscroll).min(u16::MAX as usize) as u16);
    draw_str_clipped(buf, x, y, &text[byte..], style, content);
}

/// Draw the Memory section: an address line always occupies the top content
/// row so the jump affordance is discoverable — it shows the current address
/// (with a `press : to jump` hint) when idle, and becomes the edit field with
/// a `_` cursor while `mem_input` is editing. The hex dump fills the rows
/// below it. Memory is pre-windowed by its addr, so it never applies a
/// *vertical* scroll offset; `mem_hscroll` pans it sideways (SQ-0965), and the
/// address line stays put because it is a control, not part of the dump.
///
/// When a row is wider than the pane, a horizontal scrollbar takes the bottom
/// content row (SQ-0974) — the shared `scroll` idiom, so the pan advertises
/// itself the way every other scrollable surface in the app does.
fn draw_memory(buf: &mut Buffer, content: Rect, panel: &DebugPanelState, state: &AppState, body: Style) {
    let (line, style) = match &panel.mem_input {
        Some(input) => (format!("jump: {input}_"), state.colors.theme.get("panel.border:active").style),
        None => (format!("addr: 0x{:06x}  (: jump — hex, gNN, localN, sp)", panel.mem_addr), body),
    };
    draw_str_clipped(buf, content.x, content.y, &line, style, content);
    let top = content.y + 1;
    let mut height = content.height.saturating_sub(1);
    // The pan (SQ-0965) was undiscoverable without a bar to show it existed
    // (SQ-0974): the Memory dump was the one scrollable surface in the app with
    // no scrollbar. It costs the dump its bottom row, so it appears only when
    // the content really is wider than the pane — and only when a dump row
    // survives giving one up.
    let hbar = crate::render::scroll::needs_scrollbar(
        panel.snapshot.memory_width, content.width as usize,
    ) && height >= 2;
    if hbar {
        height -= 1;
    }
    // Past the hex and its char column, each row carries the story's own text
    // for its bytes: the char column reads one ZSCII code per byte, which is
    // noise over the packed Z-characters of a dictionary key or an object short
    // name (SQ-0448/SQ-0969). Both columns are drawn — the horizontal scroll is
    // what makes the far one reachable — and a row the engine cannot anchor to a
    // string start gets nothing here rather than a plausible wrong decode.
    let ztext = state.colors.theme.get("debug.zstring").style;
    for (row, hex) in panel.snapshot.memory.iter().take(height as usize).enumerate() {
        let y = top + row as u16;
        draw_hscrolled(buf, content, y, 0, panel.mem_hscroll, hex, body);
        if let Some(Some(z)) = panel.snapshot.memory_zstrings.get(row) {
            draw_hscrolled(buf, content, y, panel.snapshot.memory_zcol, panel.mem_hscroll, z, ztext);
        }
    }
    if hbar {
        let bar = Rect::new(content.x, content.bottom() - 1, content.width, 1);
        crate::render::scroll::draw_hscrollbar(
            buf,
            bar,
            panel.snapshot.memory_width,
            content.width as usize,
            panel.mem_hscroll,
            crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme),
        );
    }
}

/// Draw the floating variable-value tooltip on top of the windows.
///
/// The box itself — placement, edge clamping, opacity, the `tooltip.*` selectors
/// — is `render::tooltip::draw_tip`, shared with the border controls' hover hint
/// (SQ-1123). This wrapper only supplies the anchor and the lines.
fn draw_tooltip(buf: &mut Buffer, area: Rect, tip: &HoverTip, state: &AppState) {
    crate::render::tooltip::draw_tip(buf, area, tip.col, tip.row, &tip.lines, &state.colors.theme, &state.symbols);
}

/// Draw the debug pane and return its window tab hit-rects as
/// `(window, tab, rect)` in absolute screen coords, so the mouse handler can
/// resolve a tab click against the exact rects rendered here.
pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer) -> Vec<(usize, usize, Rect)> {
    let Some(panel) = &state.debug else { return Vec::new() };
    // Interior fill: the standard panel surface (§2a). Transparent by default so
    // the terminal background shows through; a themed `panel.background` bg paints
    // the whole pane as a solid surface, with borders/text composing on top (their
    // transparent-bg styles patch the cell, preserving this fill).
    let surface = state.colors.theme.get("panel.background").style;
    for yy in area.top()..area.bottom() {
        for xx in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((xx, yy)) {
                c.set_symbol(" ").set_style(surface);
            }
        }
    }
    let windows = debug_panel::window_rects(area);
    let mut tab_rects: Vec<(usize, usize, Rect)> = Vec::new();
    for (i, w) in windows.iter().enumerate() {
        for (t, rect) in draw_window(buf, *w, i, panel, state).into_iter().enumerate() {
            tab_rects.push((i, t, rect));
        }
    }
    // Mouse selection (SQ-0420): reverse-video the selected cells and publish the
    // copy text read back from the drawn buffer, so a mouse-release can emit it via
    // OSC 52 — mirrors the story pane's highlight+extract. The selection lives in
    // the window's content-relative coordinates.
    if let Some((win, sel)) = panel.sel {
        if !sel.is_empty() {
            if let Some(content) = debug_panel::window_content(area, win) {
                let mut out: Vec<String> = Vec::new();
                for r in 0..content.height {
                    if let Some((c0, c1)) = crate::clipboard::row_span(content.width, sel, r as usize, 0) {
                        let mut line = String::new();
                        for c in c0..=c1 {
                            if let Some(cell) = buf.cell_mut((content.x + c, content.y + r)) {
                                let s = cell.style();
                                cell.set_style(s.add_modifier(ratatui::style::Modifier::REVERSED));
                                line.push_str(cell.symbol());
                            }
                        }
                        out.push(line.trim_end().to_string());
                    }
                }
                *panel.selection_text.borrow_mut() = Some(out.join("\n"));
            }
        }
    }
    // Tooltip paints on top of the windows.
    if let Some(tip) = &panel.hover { draw_tooltip(buf, area, tip, state); }
    tab_rects
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_text(buf: &Buffer) -> String {
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    /// SQ-0638: `clickable_spans` ranges are BYTE offsets, but the x position is
    /// a display COLUMN. "café " is 5 chars but 6 bytes ('é' is 2 bytes in
    /// UTF-8), so a byte-offset underline lands one column too far right of the
    /// "@0x001234" link text that follows it.
    #[test]
    fn underline_clickables_uses_char_column_not_byte_offset_for_multibyte_prefix() {
        let line = "café @0x001234";
        let area = Rect::new(0, 0, 40, 1);
        let mut buf = Buffer::empty(area);
        underline_clickables(&mut buf, 0, 0, line, Style::default(), Section::Objects, area);
        let underlined_cols: Vec<u16> = (0..area.width)
            .filter(|&x| buf.cell((x, 0)).unwrap().style().add_modifier.contains(Modifier::UNDERLINED))
            .collect();
        // '@' is the 6th CHAR (0-indexed column 5): c-a-f-é-space-@.
        assert_eq!(
            underlined_cols.first().copied(), Some(5),
            "underline must start at the char column of '@', not its byte offset (6)"
        );
        // And the underlined text is actually "@0x001234", not shifted.
        let underlined_text: String = underlined_cols.iter()
            .map(|&x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert_eq!(underlined_text, "@0x001234");
    }

    #[test]
    fn selection_highlights_cells_and_publishes_copy_text() {
        // SQ-0420: a selection over the Locals list (window 1, no synthetic rows)
        // reverse-videos the selected cells and publishes the logical lines as the
        // copy text read back from the drawn buffer.
        use ratatui::style::Modifier;
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[1] = crate::debug_panel::locate_section(crate::debug_panel::Section::Locals).1;
        panel.snapshot.locals = vec!["local0 = 0001".into(), "local1 = 0002".into()];
        // Window 1 (right-top) shows Locals here; select its first two content
        // rows in full (cols 0..=12 covers "local0 = 0001").
        panel.sel = Some((1, crate::clipboard::Selection {
            anchor: crate::clipboard::Point { row: 0, col: 0 },
            head: crate::clipboard::Point { row: 1, col: 12 },
        }));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let copied = state.debug.as_ref().unwrap().selection_text.borrow().clone();
        assert_eq!(copied.as_deref(), Some("local0 = 0001\nlocal1 = 0002"));
        // Window 1 content starts at (41,1); the first selected cell is reverse-video.
        let cell = buf.cell((41, 1)).expect("cell in window 1 content");
        assert!(cell.modifier.contains(Modifier::REVERSED), "selected cell must be reversed");
        // A cell outside the selection (window 0) is not reversed.
        let other = buf.cell((2, 1)).expect("cell in window 0");
        assert!(!other.modifier.contains(Modifier::REVERSED), "unselected cell not reversed");
    }

    #[test]
    fn draws_all_three_windows_default_tabs() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.snapshot.disasm = vec!["001000  add".into()];
        panel.snapshot.locals = vec!["local0 = 0001".into()];
        panel.snapshot.stack = vec!["#0 main".into()];
        panel.pc = 0x1000;
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("add"));
        assert!(text.contains("main"));
        // Tab labels for the default (first) tab of each window.
        assert!(text.contains("Disassembly"));
        assert!(text.contains("Locals"));
        assert!(text.contains("Stack"));
    }

    /// The Memory tab, jumped to `addr` and panned right by `hscroll`, drawn
    /// into an 80x24 buffer (window 2's content starts at (41, 13) and is 38
    /// columns wide). `hex_pad` sets how wide the mock's hex rows are, so a case
    /// can put the decoded-text column either inside that pane or out past its
    /// right edge — which is where a real 72-column row puts it.
    fn memory_view(addr: u32, hscroll: usize, hex_pad: usize) -> (crate::state::AppState, Buffer) {
        struct Dict(usize);
        impl crate::engine::Debugger for Dict {
            fn pc(&self) -> u32 { 0 }
            fn disassemble(&self, _a: u32, _n: usize) -> Vec<String> { Vec::new() }
            fn disassemble_raw(&self, _a: u32, _n: usize) -> Vec<String> { Vec::new() }
            fn disassemble_basic(&self, _a: u32, _n: usize) -> Vec<String> { Vec::new() }
            fn next_instr(&self, a: u32) -> u32 { a }
            fn prev_instr(&self, a: u32) -> u32 { a }
            fn executed_pcs(&self) -> std::collections::HashSet<u32> { Default::default() }
            fn stack_lines(&self) -> Vec<String> { Vec::new() }
            fn eval_stack_lines(&self) -> Vec<String> { Vec::new() }
            fn locals_lines(&self) -> Vec<String> { Vec::new() }
            fn globals_lines(&self) -> Vec<String> { Vec::new() }
            fn object_tree_lines(&self) -> Vec<String> { Vec::new() }
            fn dictionary_lines(&self) -> Vec<String> { Vec::new() }
            fn memory_hex(&self, a: u32, r: usize) -> Vec<String> {
                (0..r).map(|i| format!("{:06x}  {}", a + i as u32 * 16, ".".repeat(self.0))).collect()
            }
            fn memory_len(&self) -> u32 { 0x10000 }
            fn object_detail(&self, _o: u16) -> Vec<String> { Vec::new() }
            fn frame_locals(&self, _i: usize) -> Vec<String> { Vec::new() }
            fn var_value(&self, _v: u8) -> Option<u16> { None }
            // One entry, whose text belongs to the 0x2000 row and to no other.
            fn memory_zstrings(&self, a: u32, r: usize) -> Vec<Option<String>> {
                (0..r).map(|i| (a + i as u32 * 16 == 0x2000).then(|| "lantern".to_string())).collect()
            }
        }
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0);
        panel.goto_memory(addr, &Dict(hex_pad));
        panel.mem_hscroll = hscroll;
        state.debug = Some(panel);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);
        (state, buf)
    }

    #[test]
    fn the_memory_view_draws_the_story_text_beside_the_bytes_that_produced_it() {
        // SQ-0969: the hex row's char column reads one ZSCII code per byte, which
        // is noise over packed Z-characters. The decode belongs on the entry's
        // OWN row — a caption above the dump only restated the Dictionary tab
        // you clicked from, and never said which row it meant.
        let (state, buf) = memory_view(0x2005, 0, 2);
        // Content starts at (41, 13): row 0 is the `addr:` line, so the 0x2000
        // row is drawn at y = 14, and the decode sits two columns past a
        // 10-column hex row — x = 41 + 12.
        let row = row_text(&buf, 14);
        assert!(row.contains("002000"), "the entry's row: {row:?}");
        assert!(row.contains("lantern"), "…carries its own decoded text: {row:?}");
        assert!(!row_text(&buf, 15).contains("lantern"), "and the next row does not");

        // Themed, never hard-coded: the column's cells carry `debug.zstring`.
        let want = state.colors.theme.get("debug.zstring").style;
        let cell = buf.cell((53, 14)).expect("first cell of the Z-text column");
        assert_eq!(cell.symbol(), "l", "the column starts two past the hex row");
        assert_eq!(Some(cell.fg), want.fg, "the column takes its colour from debug.zstring");
        assert_eq!(cell.modifier, want.add_modifier, "…and its modifiers");
    }

    #[test]
    fn a_row_no_table_accounts_for_keeps_its_char_column_and_shows_no_decode() {
        // The dump also starts directly under the `addr:` line: with the caption
        // gone, no jump costs a row of hex.
        let (_, buf) = memory_view(0x3000, 0, 2);
        assert!(!buf_text(&buf).contains("lantern"), "no decode without an anchor");
        assert!(row_text(&buf, 14).contains("003000"), "and the hex owns the row");
    }

    /// FALSIFY by pinning `hscroll` to 0 inside `draw_hscrolled`: the panned
    /// case comes back with the address column still at the pane's left edge and
    /// no "lantern" anywhere on screen — the originally reported symptom, a
    /// Memory row cut off on the right with no way to see the rest of it.
    #[test]
    fn panning_brings_the_decoded_column_into_a_pane_far_too_narrow_for_it() {
        // SQ-0965: a real row is 72 columns before the decode even starts, and
        // window 2 is 38 wide — so unpanned the column is simply not on screen,
        // and the horizontal scroll is the only thing that can reach it.
        let (_, unpanned) = memory_view(0x2005, 0, 64);
        assert!(!buf_text(&unpanned).contains("lantern"), "72 columns out is off-pane");
        let (_, panned) = memory_view(0x2005, 74, 64);
        let row = win2_row(&panned, 14);
        assert!(row.starts_with("lantern"),
            "panned to the column, it reads from the pane's left edge: {row:?}");
    }

    #[test]
    fn panning_slides_the_hex_left_by_exactly_the_scroll_offset() {
        // The address itself scrolls out of view — the dump is one wide row, not
        // a fixed address gutter with a scrolling remainder.
        let (_, buf) = memory_view(0x2005, 8, 64);
        let row = win2_row(&buf, 14);
        assert!(!row.contains("002000"), "the address has scrolled off: {row:?}");
        assert!(row.starts_with("...."), "the hex body starts at the left edge: {row:?}");
        // …while the `addr:` control line above it stays put.
        assert!(win2_row(&buf, 13).contains("0x002000"), "the address line does not pan");
    }

    /// The drawn text of window 2's content on row `y` — the 80x24 layout puts
    /// its left edge at x = 41 (see `memory_view`).
    fn win2_row(buf: &Buffer, y: u16) -> String {
        row_text(buf, y).chars().skip(41).collect()
    }

    /// Window 2's content rect in the 80x24 layout `memory_view` draws into.
    fn win2_content() -> Rect {
        crate::debug_panel::window_content(Rect::new(0, 0, 80, 24), 2)
            .expect("window 2 holds content at 80x24")
    }

    /// SQ-0974: the pan existed but advertised nothing. A row wider than the
    /// pane now gets a horizontal scrollbar on the bottom content row — and a
    /// row that fits must not lose a row to one.
    ///
    /// FALSIFY by dropping the `draw_hscrollbar` call in `draw_memory`: the
    /// overflowing case comes back with a bare bottom row and no painted track
    /// anywhere — the originally reported symptom, a surface you can pan with
    /// no visible way to know it.
    #[test]
    fn a_row_wider_than_the_pane_gets_a_scrollbar_and_a_row_that_fits_does_not() {
        let content = win2_content();
        let bar_y = content.bottom() - 1;
        let bar_bgs = |buf: &Buffer| -> Vec<ratatui::style::Color> {
            (content.x..content.right()).map(|x| buf.cell((x, bar_y)).unwrap().bg).collect()
        };

        // 72-column rows in a 38-column pane: the bar is warranted.
        let (state, wide) = memory_view(0x2005, 0, 64);
        let theme = &state.colors.theme;
        let thumb = theme.get("scrollbar").style.fg.expect("scrollbar selector resolves a fill");
        let track = theme.get("scrollbar_track").style.fg.expect("scrollbar_track resolves a fill");
        let bgs = bar_bgs(&wide);
        assert!(bgs.contains(&thumb), "the thumb is painted: {bgs:?}");
        assert!(bgs.contains(&track), "the track is painted: {bgs:?}");
        assert!(
            bgs.iter().all(|c| *c == thumb || *c == track),
            "the whole bottom row is the bar, themed end to end: {bgs:?}",
        );
        // Themed, never hard-coded: the cells carry the SELECTORS' colours, and
        // the bar steals the row from the dump rather than overprinting it.
        assert!(!win2_row(&wide, bar_y).trim().contains("00"), "no hex under the bar");

        // 10-column rows in the same pane: nothing overflows, so the bottom row
        // stays a dump row.
        let (_, narrow) = memory_view(0x2005, 0, 2);
        assert!(
            bar_bgs(&narrow).iter().all(|c| *c != thumb && *c != track),
            "a pane the content fits keeps its bottom row",
        );
        assert!(
            win2_row(&narrow, bar_y).contains(':') || win2_row(&narrow, bar_y).contains("00"),
            "…and that row still carries the dump: {:?}", win2_row(&narrow, bar_y),
        );
    }

    /// The bar reports where the pan actually is: hard left unpanned, hard right
    /// once `h`/`l` have run the row out.
    #[test]
    fn the_memory_scrollbar_thumb_tracks_the_pan_at_both_ends() {
        let content = win2_content();
        let bar_y = content.bottom() - 1;
        let (state, _) = memory_view(0x2005, 0, 64);
        let thumb = state.colors.theme.get("scrollbar").style.fg.expect("scrollbar fill");
        let thumb_cols = |hscroll: usize| -> Vec<u16> {
            let (_, buf) = memory_view(0x2005, hscroll, 64);
            (content.x..content.right())
                .filter(|&x| buf.cell((x, bar_y)).unwrap().bg == thumb)
                .collect()
        };
        let left = thumb_cols(0);
        let right = thumb_cols(state.debug.as_ref().unwrap().snapshot.memory_width - 1);
        assert_eq!(left.first(), Some(&content.x), "unpanned, the thumb sits at the left edge");
        assert_eq!(
            right.last(),
            Some(&(content.right() - 1)),
            "panned to the end, it reaches the right edge",
        );
        assert!(left.last() < right.first(), "the thumb moved: {left:?} then {right:?}");
    }

    /// The drawn text of one buffer row.
    fn row_text(buf: &Buffer, y: u16) -> String {
        (buf.area.x..buf.area.right()).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect()
    }

    #[test]
    fn returns_a_tab_rect_per_window_tab_and_a_click_resolves_it() {
        // The renderer returns (window, tab, rect) for every window tab, and a
        // click inside a returned rect resolves to that exact (window, tab) —
        // mirrors the mouse handler's `debug_tabs` lookup (replaces the removed
        // `tab_at` recompute).
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        state.debug = Some(crate::debug_panel::DebugPanelState::new(0x1000));

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let tabs = draw_debug_panel(&state, area, &mut buf);

        // One entry per tab across the three windows (1 + 4 + 3). A tab whose
        // label doesn't fit the (possibly overflowing) strip gets a zero-width
        // rect — still returned, just not clickable.
        let expected: usize = crate::debug_panel::WINDOW_TABS.iter().map(|w| w.len()).sum();
        assert_eq!(tabs.len(), expected);

        // Every DRAWABLE rect resolves to its own (window, tab) when clicked at
        // its centre — the same width>0 containment test the mouse handler uses.
        let drawable: Vec<_> = tabs.iter().filter(|(_, _, r)| r.width > 0).collect();
        assert!(!drawable.is_empty(), "at least the active/fitting tabs are drawable");
        for (w, t, rect) in &drawable {
            let col = rect.x + rect.width / 2;
            let row = rect.y;
            let hit = tabs.iter().find(|(_, _, r)| {
                r.width > 0 && col >= r.x && col < r.right() && row >= r.y && row < r.bottom()
            });
            assert_eq!(hit.map(|(hw, ht, _)| (*hw, *ht)), Some((*w, *t)),
                "click at the centre of tab ({w},{t}) resolves to it");
        }
    }

    #[test]
    fn draws_a_pc_divider_row_above_the_pc_instruction() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  add".into(), "001004  sub".into()];
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        let content_y = left.y + 1; // first content row under the top border
        // content.x = left.x + 1 (border inset); divider text is drawn at
        // content.x + 1 = left.x + 2, same column as regular disasm text.
        // Row 0 is the divider (drawn ABOVE the PC line); row 1 is the actual
        // "001000  add" instruction line, shifted down by the divider.
        let divider_row: String = (left.x + 2..left.x + 2 + 10)
            .map(|x| buf.cell((x, content_y)).unwrap().symbol().to_string())
            .collect();
        assert!(divider_row.starts_with("▼── PC ──▼"), "got {divider_row:?}");
        let divider_modifier = buf.cell((left.x + 2, content_y)).unwrap().style().add_modifier;
        // Compare modifiers only: `Cell::style()` always reports concrete
        // Reset colors for unset fg/bg, so it never equals a Style::default()
        // built with `.add_modifier(...)` alone.
        assert_eq!(divider_modifier, state.colors.theme.get("debug.pc").style.add_modifier);

        let text = buf_text(&buf);
        assert!(text.contains("add"), "PC line still rendered below the divider");
    }

    #[test]
    fn marks_executed_disasm_lines_with_a_gutter_bar() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  add".into(), "001004  sub".into()];
        panel.snapshot.executed = std::collections::HashSet::from([0x1000]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        // Row 0 is the PC divider; row 1 is "001000 add" (executed); row 2 is
        // "001004 sub" (not executed).
        let content_y = left.y + 1 + 1;
        // left.x + 1 is the execution-marker gutter column (content.x); line
        // text starts one column further in.
        // Executed line (0x1000): gutter column shows the marker.
        assert_eq!(buf.cell((left.x + 1, content_y)).unwrap().symbol(), "|");
        // Not-executed line (0x1004): gutter column is blank.
        assert_eq!(buf.cell((left.x + 1, content_y + 1)).unwrap().symbol(), " ");
    }

    #[test]
    fn colours_disasm_lines_by_confidence_tier_with_executed_winning() {
        // SQ-0428: each line takes its confidence tier's style + gutter glyph,
        // and the runtime executed overlay wins over the static Soft provenance.
        use ratatui::style::Modifier;
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec![
            "001000  add".into(), // Soft + executed → executed tier wins
            "001004  sub".into(), // Soft, not executed → soft tier
            "001008  .byte 00".into(), // Data → data tier (even if executed)
            "00100c  mul".into(), // Rd, not executed → rd tier
        ];
        panel.snapshot.disasm_prov = vec![
            DisasmProvenance::Soft,
            DisasmProvenance::Soft,
            DisasmProvenance::Data,
            DisasmProvenance::Rd,
        ];
        // 0x1000 ran last turn; 0x1008 is also in the set but Data must still win.
        // Cumulative coverage (colour driver) mirrors the per-turn set here (SQ-0449).
        panel.snapshot.executed = std::collections::HashSet::from([0x1000, 0x1008]);
        panel.snapshot.executed_ever = std::collections::HashSet::from([0x1000, 0x1008]);
        state.debug = Some(panel);

        let exec = state.colors.theme.get("debug.disasm_executed");
        let soft = state.colors.theme.get("debug.disasm_soft").style;
        let data = state.colors.theme.get("debug.disasm_data").style;
        let rd = state.colors.theme.get("debug.disasm_rd").style;

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        // Row 0 is the PC divider (above the pc line); rows 1..=4 are the lines.
        // Gutter col = left.x + 1 (content.x); text col = left.x + 2.
        let (gx, tx) = (left.x + 1, left.x + 2);
        let y = left.y + 1; // first content row (the divider)

        // Executed line (0x1000, row 1): executed tier — `|` gutter + its fg,
        // NOT the soft fg (overlay beats static provenance).
        assert_eq!(buf.cell((gx, y + 1)).unwrap().symbol(), "|");
        assert_eq!(buf.cell((tx, y + 1)).unwrap().style().fg, exec.style.fg);
        assert_ne!(exec.style.fg, soft.fg, "executed tier must differ from soft");

        // Soft line (0x1004, row 2): soft fg, blank gutter.
        assert_eq!(buf.cell((gx, y + 2)).unwrap().symbol(), " ");
        assert_eq!(buf.cell((tx, y + 2)).unwrap().style().fg, soft.fg);

        // Data line (0x1008, row 3): data tier (italic) even though it is in the
        // executed set — Data outranks executed.
        assert_eq!(buf.cell((tx, y + 3)).unwrap().style().fg, data.fg);
        assert!(buf.cell((tx, y + 3)).unwrap().style().add_modifier.contains(Modifier::ITALIC),
            "data tier is italic");

        // Rd line (0x100c, row 4): plain rd fg.
        assert_eq!(buf.cell((tx, y + 4)).unwrap().style().fg, rd.fg);
    }

    #[test]
    fn cumulative_executed_colours_blue_but_only_last_turn_gets_the_bar() {
        // SQ-0449: colour (blue/executed tier) is driven by the CUMULATIVE
        // "ever executed" set; the `|` gutter is driven ONLY by the last-turn set.
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec![
            "001000  add".into(),      // ever-executed, NOT last turn → blue, no bar
            "001004  sub".into(),      // ever + last turn → blue, `|`
            "001008  .byte 00".into(), // Data + ever-executed → data tier wins colour
            "00100c  mul".into(),      // never executed → rd tier
        ];
        panel.snapshot.disasm_prov = vec![
            DisasmProvenance::Rd,
            DisasmProvenance::Rd,
            DisasmProvenance::Data,
            DisasmProvenance::Rd,
        ];
        panel.snapshot.executed_ever = std::collections::HashSet::from([0x1000, 0x1004, 0x1008]);
        panel.snapshot.executed = std::collections::HashSet::from([0x1004]); // only last turn
        state.debug = Some(panel);

        let exec = state.colors.theme.get("debug.disasm_executed").style;
        let data = state.colors.theme.get("debug.disasm_data").style;
        let rd = state.colors.theme.get("debug.disasm_rd").style;

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        let (gx, tx) = (left.x + 1, left.x + 2);
        let y = left.y + 1; // first content row (the divider)

        // (a) ever-executed but NOT last turn (0x1000): blue colour, blank gutter.
        assert_eq!(buf.cell((tx, y + 1)).unwrap().style().fg, exec.fg);
        assert_eq!(buf.cell((gx, y + 1)).unwrap().symbol(), " ", "no bar unless last turn");

        // (b) ever-executed AND last turn (0x1004): blue colour AND the `|` bar.
        assert_eq!(buf.cell((tx, y + 2)).unwrap().style().fg, exec.fg);
        assert_eq!(buf.cell((gx, y + 2)).unwrap().symbol(), "|");

        // (c) Data beats ever-executed for colour (0x1008): data fg, not blue.
        assert_eq!(buf.cell((tx, y + 3)).unwrap().style().fg, data.fg);
        assert_ne!(data.fg, exec.fg, "data tier must differ from executed");

        // Never-executed (0x100c): plain rd fg, blank gutter.
        assert_eq!(buf.cell((tx, y + 4)).unwrap().style().fg, rd.fg);
        assert_eq!(buf.cell((gx, y + 4)).unwrap().symbol(), " ");
    }

    #[test]
    fn shows_a_non_default_active_tab_and_hides_the_others_content() {
        let mut state = crate::state::AppState::default();
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[1] = crate::debug_panel::locate_section(crate::debug_panel::Section::Locals).1; // a non-default tab
        panel.snapshot.locals = vec!["local0 = 0001".into()];
        panel.snapshot.globals = vec!["g00 = should-not-show".into()];
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("local0 = 0001"));
        assert!(!text.contains("should-not-show"));
    }

    #[test]
    fn objects_show_disclosure_triangles_and_no_underline() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[1] = crate::debug_panel::locate_section(crate::debug_panel::Section::Objects).1; // Objects tab
        panel.snapshot.objects = vec!["[1] lamp".into(), "[2] rock".into()];
        panel.expanded_objects = std::collections::HashSet::from([1u16]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("▼"), "expanded object gets a ▼ marker");
        assert!(text.contains("▶"), "collapsed object gets a ▶ marker");

        // The tree line is no longer underlined: the first glyph of "[1] lamp"
        // (drawn just after the "▼ " marker) carries no UNDERLINED modifier.
        let [_, top, _] = crate::debug_panel::window_rects(area);
        let content = Rect::new(top.x + 1, top.y + 1, top.width.saturating_sub(2), top.height.saturating_sub(2));
        let cell = buf.cell((content.x + 2, content.y)).unwrap(); // past "▼ "
        assert_eq!(cell.symbol(), "[");
        assert!(!cell.style().add_modifier.contains(Modifier::UNDERLINED), "tree line must not be underlined");
    }

    /// SQ-0975: the expanded detail's `entry @0x……` is a real link — the only
    /// route to the §12.3 entry now the tree row points at the property table —
    /// so it draws underlined like every other address link.
    #[test]
    fn an_expanded_objects_detail_underlines_its_entry_address_link() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[1] = crate::debug_panel::locate_section(crate::debug_panel::Section::Objects).1;
        panel.snapshot.objects = vec!["@0x000340 [1] lamp".into()];
        panel.expanded_objects = std::collections::HashSet::from([1u16]);
        panel.snapshot.object_details.insert(1, vec!["entry @0x000110".into()]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [_, top, _] = crate::debug_panel::window_rects(area);
        let content = Rect::new(top.x + 1, top.y + 1, top.width.saturating_sub(2), top.height.saturating_sub(2));
        // Detail row (y + 1) draws under a 4-column indent; "entry " is 6 more.
        let x = content.x + 4 + 6;
        let cell = buf.cell((x, content.y + 1)).unwrap();
        assert_eq!(cell.symbol(), "@", "the entry link starts here");
        assert!(
            cell.style().add_modifier.contains(Modifier::UNDERLINED),
            "the entry address underlines like the link it is",
        );
    }

    #[test]
    fn callstack_shows_disclosure_triangles_and_indents_expanded_locals() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        // Window 2 tab 0 = Call Stack (the default).
        panel.snapshot.stack = vec![
            "#0  fn@004a00  ret=004a35  args=2".into(),
            "#1  fn@005000  ret=005035  args=0".into(),
        ];
        panel.expanded_frames = std::collections::HashSet::from([0usize]);
        panel.snapshot.frame_details = std::collections::HashMap::from([
            (0usize, vec!["local0 = 0x0001  (1)".to_string()]),
        ]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("▼"), "expanded frame gets a ▼ marker");
        assert!(text.contains("▶"), "collapsed frame gets a ▶ marker");
        assert!(text.contains("local0 = 0x0001  (1)"), "expanded frame's locals appear");

        // The frame text draws at content.x + 2 (past the "▼ " marker); its first
        // glyph is '#'. The detail row below (row 1) is indented by four spaces.
        let [_, _, bot] = crate::debug_panel::window_rects(area);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        assert_eq!(buf.cell((content.x + 2, content.y)).unwrap().symbol(), "#");
        assert_eq!(buf.cell((content.x, content.y)).unwrap().symbol(), "▼");
        // Detail row (row 1) indented: first four cells blank, then the local.
        assert_eq!(buf.cell((content.x + 4, content.y + 1)).unwrap().symbol(), "l");
    }

    #[test]
    fn draws_the_hover_tooltip_value_text() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  loadw g0f -> sp".into()];
        panel.hover = Some(crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 5, 5));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("g0f = 0x1234"), "tooltip value text is painted");
    }

    #[test]
    fn hover_tooltip_is_opaque_and_clears_underline_underneath() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  loadw g0f -> sp".into()];
        panel.hover = Some(crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 5, 5));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        // Pre-underline everything, as clickable-span underlines would leave the
        // disasm cells beneath the tooltip.
        for cell in buf.content.iter_mut() {
            cell.set_style(Style::new().add_modifier(Modifier::UNDERLINED));
        }
        draw_debug_panel(&state, area, &mut buf);

        // FIND the text rather than pinning where it lands: the box is centred on
        // its anchor and sits clear of the pointer row (SQ-1139), so a hard-coded
        // cell here only tests that the geometry has not changed — which is not
        // what this case is about. What it IS about is that whatever cell the 'g'
        // of "g0f = …" ends up in carries no underline bled through from beneath.
        // Matched on "g0f = " and not on "g0f": the DISASM line under the tooltip
        // is `loadw g0f -> sp`, which contains the operand too and is underlined
        // exactly as this case arranged. A three-character probe finds that one
        // first and then asserts the bleed it was looking for — a test that fails
        // for the right-sounding reason on the wrong cell.
        let needle: Vec<char> = "g0f = ".chars().collect();
        let (gx, gy) = (0..area.height)
            .find_map(|y| {
                (0..area.width.saturating_sub(needle.len() as u16)).find_map(|x| {
                    needle
                        .iter()
                        .enumerate()
                        .all(|(i, want)| {
                            buf.cell((x + i as u16, y)).map(|c| c.symbol()) == Some(&want.to_string())
                        })
                        .then_some((x, y))
                })
            })
            .expect("the tooltip's value text is painted somewhere");
        let cell = buf.cell((gx, gy)).expect("cell in bounds");
        assert_eq!(cell.symbol(), "g", "tooltip text painted here");
        assert!(!cell.style().add_modifier.contains(Modifier::UNDERLINED),
            "underline underneath must not bleed through the tooltip");
    }

    #[test]
    fn hover_tooltip_clamps_at_the_bottom_right_corner_without_panicking() {
        let state = crate::state::AppState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        // Anchor at the very bottom-right corner: the box must flip/shift inside
        // `area` rather than draw out of bounds and panic.
        let tip = crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), area.right() - 1, area.bottom() - 1);
        draw_tooltip(&mut buf, area, &tip, &state);
        assert!(buf_text(&buf).contains("g0f = 0x1234"), "clamped tooltip still paints");
    }

    #[test]
    fn hover_tooltip_skips_a_too_small_area_without_panicking() {
        let state = crate::state::AppState::default();
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        let tip = crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 0, 0);
        draw_tooltip(&mut buf, area, &tip, &state); // must not panic
    }
}
