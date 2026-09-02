//! Hints panel modal overlay — a centered mini-terminal for the hint session.
//!
//! When `AppState.hints` is `Some(HintSession)`, `draw_hints_panel` renders a
//! dialog with the hint session transcript, an optional built-in-HINT suggestion
//! line, and an input row.  It mirrors the `draw_gallery`/`draw_reset_dialog`
//! pattern: dialog chrome via `draw_dialog`, content drawn into `rects.content`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::render::transcript::wrap_line;
use crate::state::AppState;

// Minimum dimensions for the hints panel.
const MIN_W: u16 = 40;
const MIN_H: u16 = 10;

// ── HintsPanelRects ───────────────────────────────────────────────────────────

/// Hit-rects returned by `draw_hints_panel` for mouse event routing.
pub struct HintsPanelRects {
    /// Full dialog area (border included).
    pub area: Rect,
    /// The `[X]` close button, if rendered.
    pub close: Option<Rect>,
    /// The bottom `[Close]` dialog button, if rendered.
    pub close_button: Option<Rect>,
    /// The input row inside the dialog content area.
    pub input: Rect,
    /// Maximum transcript scroll offset for this render (wrapped lines minus the
    /// visible body height). Used to clamp wheel-driven scrolling.
    pub max_scroll: u16,
}

// ── draw_hints_panel ──────────────────────────────────────────────────────────

/// Draw the Hints panel so it fills `area` exactly (experiment: the panel is
/// laid over the story pane, taking its whole rect rather than floating as a
/// centered modal). Because `area` is the live story-pane rect recomputed each
/// frame, the panel tracks terminal resizes automatically.
///
/// Returns `None` when `state.overlays.hints` is `None` or the area is too small.
/// Returns `Some(HintsPanelRects)` with hit-rects for the close button and
/// the input row.
pub fn draw_hints_panel(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<HintsPanelRects> {
    let session = state.overlays.hints.as_ref()?;

    if area.width < MIN_W || area.height < MIN_H {
        return None;
    }

    // Build DialogStyle from state colors (mirrors gallery.rs / reset_dialog.rs).
    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[DialogButton { id: ButtonId::Close, label: "Close" }];
    let spec = DialogSpec {
        title: &session.label,
        placement: Placement::Positioned(area),
        buttons,
        show_close: true,
        default: Some(ButtonId::Close),
        focus: None,
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;
    let close_button = rects
        .buttons
        .iter()
        .find(|(id, _)| *id == ButtonId::Close)
        .map(|(_, r)| *r);

    if content.height == 0 || content.width == 0 {
        return Some(HintsPanelRects {
            area: rects.area,
            close: rects.close,
            close_button,
            input: Rect::new(content.x, content.y, content.width, 0),
            max_scroll: 0,
        });
    }

    // Pull the companion VM's screen model once for this frame (immutable read):
    // its upper (grid) window is the InvisiClues split-screen menu we draw above
    // the clue text, and whether it awaits a keypress decides the input prompt.
    let crate::state::HintSource::Zcode(vm) = &session.source;
    let char_mode = matches!(vm.pending_input(), crate::session::InputKind::Char);
    let honor = state.config.honor_game_colours;
    let companion = crate::session::screen_model_from_machine(&vm.machine);

    // The last row of content is always the input row.
    // Everything above it is the transcript area (possibly preceded by the
    // built-in-HINT suggestion line, and topped by the companion menu window).
    let input_y = content.bottom().saturating_sub(1);
    let input_rect = Rect::new(content.x, input_y, content.width, 1);

    // Draw the input row. In char mode the companion navigates by single
    // keypresses, so show a dim nav hint; in line mode keep the "> <input>" caret.
    let input_style = state.colors.theme.get("dialog.background").style;
    if char_mode {
        let muted = input_style.patch(state.colors.theme.get("muted").style);
        let prompt = "(press a key \u{00b7} \u{2191}/\u{2193}/Enter navigate \u{00b7} Esc close)";
        crate::render::draw_str_clipped(buf, content.x, input_y, prompt, muted, content);
    } else {
        let input_line = format!("> {}", session.input);
        crate::render::draw_str_clipped(buf, content.x, input_y, &input_line, input_style, content);
    }

    // The transcript display area: content rows above the input row.
    if content.height < 2 {
        return Some(HintsPanelRects { area: rects.area, close: rects.close, close_button, input: input_rect, max_scroll: 0 });
    }
    let mut transcript_area = Rect::new(content.x, content.y, content.width, content.height - 1);

    // Companion upper (grid) menu window: draw it at the top of the content,
    // above the lower clue text, and shrink the transcript body by the rows it
    // consumes. Text-only hint files have no grid → skipped, leaving the panel
    // rendering exactly as before. Cap the menu height so the input row plus at
    // least two transcript rows always survive a tall menu.
    if let Some(grid) = companion.grid() {
        if grid.active_rows > 0 && transcript_area.height > 2 {
            let upper_cap = transcript_area.height - 2;
            let upper_rect =
                Rect::new(transcript_area.x, transcript_area.y, transcript_area.width, upper_cap);
            let mut links: Vec<((u16, u16), u32)> = Vec::new();
            let used = crate::render::upper_window::draw_upper_window(
                grid, char_mode, &state.colors, upper_rect, buf, honor, &mut links,
            );
            transcript_area = Rect::new(
                transcript_area.x,
                transcript_area.y + used,
                transcript_area.width,
                transcript_area.height - used,
            );
        }
    }

    // Draw the content, bottom-up from the input row:
    //   (a) builtin_hint suggestion line (if set) — topmost reserved row.
    //   (b) wrapped transcript lines, scrolled by session.scroll.

    // Decide how many rows the built-in hint line occupies (0 or 1).
    let hint_row_count: u16 = if session.builtin_hint { 1 } else { 0 };

    // Draw built-in hint suggestion on the very first content row (row 0 of transcript_area).
    if session.builtin_hint && transcript_area.height >= 1 {
        let suggestion = "This game has its own hints \u{2014} type HINT in the story.";
        let dim_style = state.colors.theme.get("dialog.background").style
            .patch(state.colors.theme.get("dialog.hint_suggestion").style);
        crate::render::draw_str_clipped(
            buf,
            transcript_area.x,
            transcript_area.y,
            suggestion,
            dim_style,
            transcript_area,
        );
    }

    // Transcript body area: below the hint suggestion line.
    if transcript_area.height <= hint_row_count {
        return Some(HintsPanelRects { area: rects.area, close: rects.close, close_button, input: input_rect, max_scroll: 0 });
    }
    let body_top = transcript_area.y + hint_row_count;
    let body_h = transcript_area.bottom() - body_top;
    let body_area = Rect::new(transcript_area.x, body_top, transcript_area.width, body_h);

    // Word-wrap each logical transcript line to the content width, then display
    // the window of `body_h` rows honoring `session.scroll`.
    let wrapped: Vec<String> = session
        .transcript
        .iter()
        .flat_map(|line| wrap_line(line, body_area.width))
        .collect();

    let n = wrapped.len();
    let rows = body_h as usize;
    let max_scroll = n.saturating_sub(rows).min(u16::MAX as usize) as u16;
    // Use the eased (animated) offset for display; the logical target drives max.
    let scroll = (session.effective_scroll() as usize).min(max_scroll as usize);

    // Reserve a 1-col gutter for the scrollbar when the transcript overflows.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(n, rows) && body_area.width >= 2;
    let text_w = if scrollbar_visible { body_area.width.saturating_sub(1) } else { body_area.width };
    let text_area = Rect::new(body_area.x, body_area.y, text_w, body_area.height);

    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    let visible = &wrapped[start..end];

    let body_style = state.colors.theme.get("dialog.background").style;
    for (i, line) in visible.iter().enumerate() {
        let row_y = body_top + i as u16;
        if row_y >= text_area.bottom() {
            break;
        }
        crate::render::draw_str_clipped(buf, text_area.x, row_y, line, body_style, text_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(body_area.right().saturating_sub(1), body_area.y, 1, body_area.height);
        // `start` is the index of the first visible row (0 = oldest/top).
        let look = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme);
        crate::render::scroll::draw_scrollbar(buf, sb_area, n, rows, start, look);
    }

    Some(HintsPanelRects { area: rects.area, close: rects.close, close_button, input: input_rect, max_scroll })
}

// ── Hints panel keyboard routing ──────────────────────────────────────────────

/// Routing decision for a key pressed while the hints panel is open.
pub enum HintKeyKind {
    /// Close the hints panel (Esc).
    Close,
    /// Scroll the hint transcript by this many lines (positive = toward older
    /// content), instead of forwarding the key to the companion VM. Reserves the
    /// navigation keys for the panel so an InvisiClues `read_char` prompt (which
    /// only wants its own letter keys, e.g. H/Q) never sees a stray arrow and
    /// reprints with a spurious line-feed.
    Scroll(i32),
    /// Route the key to the hint sub-session.
    ToSession,
}

/// Lines scrolled per PageUp/PageDown in the hints panel.
const HINT_PAGE_LINES: i32 = 10;

/// Map a key code to a HintKeyKind.
/// Esc → Close; everything else → ToSession.
pub fn hint_key_routes(code: crossterm::event::KeyCode) -> HintKeyKind {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => HintKeyKind::Close,
        // Only PageUp/PageDown scroll the clue (lower) window; the arrow keys and
        // everything else are forwarded to the companion VM so its upper-window
        // menu stays navigable. (A no-output menu keystroke no longer drops a blank
        // line into the clue window — see HintSession::apply_turn.)
        KeyCode::PageUp => HintKeyKind::Scroll(HINT_PAGE_LINES),
        KeyCode::PageDown => HintKeyKind::Scroll(-HINT_PAGE_LINES),
        _ => HintKeyKind::ToSession,
    }
}

/// What a `ToSession` key should do, decided by the companion VM's pending input
/// mode. In `Char` mode (an InvisiClues `read_char` menu) every key is forwarded
/// to the VM; in `Line` mode the key edits the local input buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintInputAct {
    /// Char mode: forward the keypress to the companion VM (menu navigation).
    ForwardKey,
    /// Line mode Enter: submit the accumulated input line to the VM.
    SubmitLine,
    /// Line mode Backspace: drop the last input char.
    BufferPop,
    /// Line mode printable key: push it into the input buffer.
    BufferPush(char),
    /// No effect (e.g. an arrow/function key during a line read).
    Ignore,
}

/// Decide what a `ToSession` keypress does given the companion VM's pending
/// input `kind`. Char mode forwards every key; line mode edits the buffer.
/// (Esc never reaches here — it routes to [`HintKeyKind::Close`].)
pub fn hint_input_action(
    kind: crate::session::InputKind,
    code: crossterm::event::KeyCode,
) -> HintInputAct {
    use crossterm::event::KeyCode;
    if kind == crate::session::InputKind::Char {
        return HintInputAct::ForwardKey;
    }
    match code {
        KeyCode::Enter => HintInputAct::SubmitLine,
        KeyCode::Backspace => HintInputAct::BufferPop,
        KeyCode::Char(c) => HintInputAct::BufferPush(c),
        _ => HintInputAct::Ignore,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a minimal `HintSession` backed by the minizork.z3 fixture.
    ///
    /// The fixture path is resolved relative to CARGO_MANIFEST_DIR (the app
    /// crate root). If the fixture is absent the test is skipped (the helper
    /// returns `None`).
    fn make_hint_session() -> Option<crate::state::HintSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None;
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        Some(crate::state::HintSession {
            source: crate::state::HintSource::Zcode(session),
            transcript: vec!["pick a topic".to_string()],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: "3".to_string(),
            label: "Hints: X".to_string(),
            builtin_hint: true,
        })
    }

    #[test]
    fn hints_panel_renders_title_transcript_suggestion_and_input() {
        let Some(hint_session) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };

        let mut state = crate::state::AppState::default();
        state.overlays.hints = Some(hint_session);

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;

        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let r = rects.expect("draw_hints_panel should return rects when hints is Some");
        assert!(r.close.is_some(), "close button rect should be present");

        // Collect all rendered chars into a flat string for assertions.
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars())
            .collect();

        assert!(all.contains("Hints: X"), "title 'Hints: X' must appear in the buffer");
        assert!(all.contains("pick a topic"), "transcript text must appear in the buffer");
        assert!(all.contains("HINT"), "built-in hint suggestion ('type HINT') must appear");
        assert!(all.contains("3"), "input '3' must appear in the buffer");
    }

    /// The companion VM's upper (grid) window — the InvisiClues split-screen
    /// menu — must render above the lower clue transcript. We paint a known
    /// 2-cell menu ("ZQ") into the companion's upper window and assert it
    /// appears on a row strictly above the transcript text.
    #[test]
    fn hints_panel_draws_companion_upper_window_above_transcript() {
        let Some(mut hint_session) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };

        // Paint a known 2-row upper window into the companion session.
        let crate::state::HintSource::Zcode(vm) = &mut hint_session.source;
        vm.machine.screen.upper.resize(2, 8);
        vm.machine.screen.upper.put(1, 1, 'Z', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        vm.machine.screen.upper.put(1, 2, 'Q', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        vm.machine.screen.upper_window_rows = 2;

        let mut state = crate::state::AppState::default();
        state.overlays.hints = Some(hint_session);

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();

        // Reconstruct per-row strings from the rendered buffer.
        let buf = terminal.backend().buffer();
        let w = buf.area.width as usize;
        let mut rows: Vec<String> = vec![String::new(); buf.area.height as usize];
        for (i, cell) in buf.content().iter().enumerate() {
            rows[i / w].push_str(cell.symbol());
        }

        let upper_row = rows.iter().position(|r| r.contains("ZQ"))
            .expect("companion upper-window menu 'ZQ' must render in the panel");
        let text_row = rows.iter().position(|r| r.contains("pick a topic"))
            .expect("transcript text must still render");
        assert!(
            upper_row < text_row,
            "upper-window menu (row {upper_row}) must be above the transcript (row {text_row})"
        );
    }

    #[test]
    fn hints_panel_returns_none_when_no_session() {
        let state = crate::state::AppState::default(); // hints = None
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;
        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();
        assert!(rects.is_none(), "draw_hints_panel must return None when hints is None");
    }

    #[test]
    fn hints_panel_returns_none_on_small_terminal() {
        let Some(hint_session) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        let mut state = crate::state::AppState::default();
        state.overlays.hints = Some(hint_session);

        let backend = TestBackend::new(20, 5); // too small
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;
        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();
        assert!(rects.is_none(), "draw_hints_panel must return None on very small terminals");
    }

    // ── hint_key_routes ───────────────────────────────────────────────────────

    #[test]
    fn hint_panel_keys_close_on_esc_else_route() {
        use crossterm::event::KeyCode;
        assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Close));
        assert!(matches!(hint_key_routes(KeyCode::Char('a')), HintKeyKind::ToSession));
    }

    /// Regression: Enter must route to the hint session input (ToSession), not Close.
    /// The hints panel has a text input; Enter submits that input regardless of any
    /// default-button decoration on the Close button.
    #[test]
    fn hints_enter_submits_input_not_close() {
        use crossterm::event::KeyCode;
        let routed = hint_key_routes(KeyCode::Enter);
        assert!(
            matches!(routed, HintKeyKind::ToSession),
            "Enter must be routed to the hint session input (ToSession), not Close"
        );
    }

    // ── hint_input_action (char/line routing) ─────────────────────────────────

    /// Only PageUp/PageDown scroll the clue window; the arrow keys (and everything
    /// else) route to the companion VM so its upper-window menu stays navigable.
    #[test]
    fn hint_key_routes_pagekeys_scroll_arrows_go_to_session() {
        use crossterm::event::KeyCode;
        assert!(matches!(hint_key_routes(KeyCode::PageUp), HintKeyKind::Scroll(d) if d > 0));
        assert!(matches!(hint_key_routes(KeyCode::PageDown), HintKeyKind::Scroll(d) if d < 0));
        // Arrows drive the companion's menu — forwarded, not scrolled.
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
                     KeyCode::Home, KeyCode::End, KeyCode::Char('h'), KeyCode::Enter] {
            assert!(matches!(hint_key_routes(code), HintKeyKind::ToSession),
                "{code:?} must reach the companion VM");
        }
        assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Close));
    }

    /// In Char mode (an InvisiClues `read_char` menu) every key that reaches the
    /// session is forwarded to the companion VM — arrows drive the menu, letters/
    /// Enter/Backspace act — never buffered. (PageUp/PageDown are handled upstream.)
    #[test]
    fn hint_input_action_char_mode_forwards_keys() {
        use crossterm::event::KeyCode;
        use crate::session::InputKind::Char;
        for code in [
            KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
            KeyCode::Enter, KeyCode::Backspace, KeyCode::Char('h'), KeyCode::F(1),
        ] {
            assert_eq!(
                hint_input_action(Char, code),
                HintInputAct::ForwardKey,
                "char mode must forward {code:?} to the VM (menu nav), not buffer it"
            );
        }
    }

    /// In Line mode (a plain text hint prompt) the key edits the local buffer:
    /// Enter submits, Backspace pops, a printable pushes.
    #[test]
    fn hint_input_action_line_mode_edits_buffer() {
        use crossterm::event::KeyCode;
        use crate::session::InputKind::Line;
        assert_eq!(hint_input_action(Line, KeyCode::Enter), HintInputAct::SubmitLine);
        assert_eq!(hint_input_action(Line, KeyCode::Backspace), HintInputAct::BufferPop);
        assert_eq!(hint_input_action(Line, KeyCode::Char('x')), HintInputAct::BufferPush('x'));
    }

    /// End-to-end on a booted companion (which boots to `Line` mode): a printable
    /// key routes to `BufferPush`, and applying it grows `hs.input` — i.e. a line
    /// hint still buffers rather than driving the VM.
    #[test]
    fn hint_line_mode_char_buffers_into_input() {
        use crossterm::event::KeyCode;
        let Some(mut hs) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        hs.input.clear();
        let crate::state::HintSource::Zcode(vm) = &hs.source;
        assert_eq!(vm.pending_input(), crate::session::InputKind::Line, "companion boots to a line read");

        // Route a printable key as the event loop would, then apply the action.
        match hint_input_action(vm.pending_input(), KeyCode::Char('q')) {
            HintInputAct::BufferPush(c) => hs.input.push(c),
            other => panic!("line-mode printable must buffer, got {other:?}"),
        }
        assert_eq!(hs.input, "q", "line-mode key buffers into hs.input");
    }
}

