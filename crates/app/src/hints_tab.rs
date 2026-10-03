//! The Journal's Hints tab (SQ-1685): the story's InvisiClues-style hint file,
//! run as a Z-machine companion in a window of its own inside the Journal.
//!
//! This replaces the centred modal the hint session used to be. The session
//! itself is unchanged — `AppState.overlays.hints`, a [`HintSession`] — but it
//! no longer covers the story:
//!
//! * it **starts by itself** the first time the tab is shown ([`ensure_started`]),
//!   **survives tab switches** (the same session object is drawn again, so you
//!   return to where you were in the hint menu), is discarded on quit and never
//!   reaches the save archive;
//! * with **no hint file** the tab says so and offers `[ Download hints… ]`
//!   (`download-hints`, which works in the game as well as in the story browser);
//! * the keyboard stays in the story until a click lands inside the hint window
//!   (or Tab reaches it, see [`AppState::cycle_focus`]). While the session has it,
//!   every key — arrows and single characters included, because InvisiClues is a
//!   `read_char` menu — goes to the hint VM ([`on_key`]); the focus is always
//!   visible, because only the focused input shows a cursor and the other is dimmed.
//!
//! Selectors: `journal.hints.transcript`, `.input`, `.input:unfocused`,
//! `.builtin`, `.nohint`, `.button`, and `.tab:focused` (the tab label's mark,
//! drawn by [`crate::journal::draw_tab_bar`]).

use std::cell::{Cell, RefCell};
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::engine::Engine;
use crate::hint_download::{HintDlOutcome, HintDownloader};
use crate::host::hints::NO_HINT_MESSAGE;
use crate::render::transcript::wrap_line;
use crate::state::{AppState, Focus, HintSession, HintSource};

/// The registry command the tab's button runs.
pub const CMD_DOWNLOAD: &str = "download-hints";
const DOWNLOAD_LABEL: &str = " [ Download hints… ] ";
const BUILTIN_LINE: &str = "This game has its own hints \u{2014} type HINT in the story.";
/// Lines scrolled per PageUp/PageDown while the session has the keyboard.
const HINT_PAGE_LINES: i32 = 10;

/// Where the tab's session stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    /// Not tried yet: the next time the tab is on screen, [`ensure_started`] opens it.
    #[default]
    NotStarted,
    /// `overlays.hints` holds the running session.
    Running,
    /// Resolution found no hint file (a missing one and an ambiguous pair both
    /// land here: today's `host::hints::open` answers `Ok(None)` for both).
    NoHint,
    /// A hint file resolved but would not boot; the reason.
    Failed(String),
    /// The hint file quit (`@quit`). Showing the tab again starts it afresh.
    Ended,
}

/// Where the last draw put things, for routing the mouse and clamping scroll.
#[derive(Default, Clone, Debug)]
pub struct HintsHits {
    /// The whole tab body: a click anywhere in the running window is "inside the hint window".
    pub area: Rect,
    /// The input row.
    pub input: Rect,
    /// The `[ Download hints… ]` button.
    pub download: Option<Rect>,
    /// Largest transcript scroll offset of the last draw.
    pub max_scroll: u16,
}

/// The tab's own state; the session is `AppState.overlays.hints`.
pub struct HintsTab {
    pub phase: Phase,
    /// What the button or a download last did, shown under the no-hint message.
    pub message: Option<String>,
    downloader: HintDownloader,
    hits: RefCell<HintsHits>,
    max_scroll: Cell<u16>,
}

impl Default for HintsTab {
    fn default() -> Self {
        HintsTab {
            phase: Phase::NotStarted,
            message: None,
            downloader: HintDownloader::new(),
            hits: RefCell::new(HintsHits::default()),
            max_scroll: Cell::new(0),
        }
    }
}

impl std::fmt::Debug for HintsTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintsTab").field("phase", &self.phase).field("message", &self.message).finish()
    }
}

impl HintsTab {
    /// Where the last draw put things.
    pub fn hits(&self) -> HintsHits {
        self.hits.borrow().clone()
    }
}

// ── Starting, ending, downloading ────────────────────────────────────────────

/// Start the session the first time the tab is on screen. Called once per loop
/// turn; `true` when something changed and a redraw is due.
///
/// Resolution is [`crate::host::hints::open`] unchanged — this only decides WHEN.
pub fn ensure_started(state: &mut AppState, story_path: &Path, ifid: &str) -> bool {
    if !state.hints_tab_visible() || state.hints_tab.phase != Phase::NotStarted {
        return false;
    }
    let index = crate::hints::load_hint_index(&state.config.user_dir);
    match crate::host::hints::open(story_path, ifid, &state.title, &index, &state.dict_words, &state.config) {
        Ok(Some(session)) => {
            state.overlays.hints = Some(session);
            state.hints_tab.phase = Phase::Running;
        }
        Ok(None) => state.hints_tab.phase = Phase::NoHint,
        Err(e) => state.hints_tab.phase = Phase::Failed(e.to_string()),
    }
    true
}

/// `open-hints` / Action::OpenHints: show the Hints tab and start its session if
/// it has not been. Like the tab click and Alt+4 it does NOT move the keyboard.
pub fn show(state: &mut AppState, mapper: &mut mapper::mapper::Mapper, story_path: &Path, ifid: &str) {
    crate::input::apply_action(crate::input::Action::SetJournalTab(crate::journal::JournalTab::Hints), state, mapper);
    ensure_started(state, story_path, ifid);
}

/// A finished session (`@quit`): drop it and give the keyboard back.
fn end_session(state: &mut AppState) {
    state.overlays.hints = None;
    state.hints_tab.phase = Phase::Ended;
    if state.focus == Focus::Hints {
        state.focus = Focus::Game;
    }
}

/// `download-hints` for the running game: fetch a matching InvisiClues file beside
/// the story. The result arrives through [`poll_download`].
pub fn start_download(state: &mut AppState, story_path: &Path) {
    let line = if state.hints_tab.downloader.busy() {
        "Already downloading hints…".to_string()
    } else if state.hints_tab.phase == Phase::Running {
        "This story already has a hint file".to_string()
    } else {
        match crate::hints::hint_download_for(&state.ifid) {
            None => "No InvisiClues found for this story".to_string(),
            Some(dl) => {
                let dest = story_path.with_file_name(&dl.filename);
                let title = story_path.file_stem().and_then(|s| s.to_str()).unwrap_or("this story").to_owned();
                state.hints_tab.downloader.start(
                    dl.url,
                    dest,
                    story_path.to_path_buf(),
                    state.source.disk_entry.clone(),
                    title,
                );
                "Downloading hints…".to_string()
            }
        }
    };
    state.hints_tab.message = Some(line.clone());
    state.set_status(line);
}

/// Drain finished downloads. A completed one wrote a sidecar beside the story, so
/// the tab asks to resolve again. `true` when a redraw is due.
pub fn poll_download(state: &mut AppState) -> bool {
    let mut changed = false;
    for r in state.hints_tab.downloader.drain() {
        changed = true;
        let line = match r.outcome {
            HintDlOutcome::Done => {
                state.hints_tab.phase = Phase::NotStarted;
                format!("Downloaded hints for {}", r.title)
            }
            HintDlOutcome::Failed(msg) => format!("Hint download failed: {msg}"),
        };
        state.hints_tab.message = Some(line.clone());
        state.set_status(line);
    }
    changed || state.hints_tab.downloader.busy()
}

/// Is a hint download in flight? (The run loop keeps polling while it is.)
pub fn download_busy(state: &AppState) -> bool {
    state.hints_tab.downloader.busy()
}

// ── Keyboard ─────────────────────────────────────────────────────────────────

/// Routing decision for a key pressed while the hint session has the keyboard.
pub enum HintKeyKind {
    /// Hand the keyboard back to the story (Esc).
    Leave,
    /// Scroll the hint transcript by this many lines (positive = toward older
    /// content) instead of forwarding the key. Reserves PageUp/PageDown for the
    /// tab so an InvisiClues `read_char` prompt never sees a stray page key.
    Scroll(i32),
    /// Route the key to the hint sub-session.
    ToSession,
}

/// Map a key code to a [`HintKeyKind`]. Esc leaves; PageUp/PageDown scroll; the
/// arrows and everything else go to the companion VM so its menu stays navigable.
pub fn hint_key_routes(code: KeyCode) -> HintKeyKind {
    match code {
        KeyCode::Esc => HintKeyKind::Leave,
        KeyCode::PageUp => HintKeyKind::Scroll(HINT_PAGE_LINES),
        KeyCode::PageDown => HintKeyKind::Scroll(-HINT_PAGE_LINES),
        _ => HintKeyKind::ToSession,
    }
}

/// What a `ToSession` key does, decided by the companion VM's pending input mode.
/// In `Char` mode (an InvisiClues `read_char` menu) every key is forwarded to the
/// VM; in `Line` mode the key edits the local input buffer.
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
pub fn hint_input_action(kind: crate::session::InputKind, code: KeyCode) -> HintInputAct {
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

/// Did [`on_key`] take the key?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Consumed: do not route it any further.
    Handled,
    /// Not the hint session's: route it as usual (Ctrl/Alt chords — quit, the tab
    /// keys, the palette — keep working while the hint input is focused).
    PassThrough,
}

/// Route a key while the hint session has the keyboard. Does nothing and says
/// `PassThrough` when it does not ([`AppState::hints_have_keyboard`]).
///
/// * Esc, and Tab / Shift-Tab on an EMPTY input, hand the keyboard back (Tab steps
///   [`AppState::cycle_focus`], so Shift-Tab reverses). A char-mode menu has no
///   input text, so it counts as empty.
/// * PageUp/PageDown scroll the transcript.
/// * Everything else is the hint VM's, arrows and Enter included.
pub fn on_key(state: &mut AppState, key: KeyEvent) -> KeyOutcome {
    if !state.hints_have_keyboard() {
        return KeyOutcome::PassThrough;
    }
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return KeyOutcome::PassThrough;
    }
    let anim = state.config.animation.clone();
    let max = state.hints_tab.max_scroll.get();
    let Some(hs) = state.overlays.hints.as_mut() else {
        return KeyOutcome::PassThrough;
    };
    let kind = {
        let HintSource::Zcode(vm) = &hs.source;
        vm.pending_input()
    };
    let input_empty = kind == crate::session::InputKind::Char || hs.input.is_empty();
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
        if input_empty {
            state.cycle_focus(key.code == KeyCode::Tab);
        }
        return KeyOutcome::Handled;
    }
    match hint_key_routes(key.code) {
        HintKeyKind::Leave => state.focus = Focus::Game,
        HintKeyKind::Scroll(delta) => hs.scroll_by(delta, max, &anim),
        HintKeyKind::ToSession => {
            let result = match hint_input_action(kind, key.code) {
                HintInputAct::ForwardKey => crate::engine::key_event_to_input(key).and_then(|ki| {
                    let HintSource::Zcode(vm) = &mut hs.source;
                    vm.submit_key(ki)
                }),
                HintInputAct::SubmitLine => {
                    let line = std::mem::take(&mut hs.input);
                    let HintSource::Zcode(vm) = &mut hs.source;
                    Some(vm.submit(&line))
                }
                HintInputAct::BufferPop => {
                    hs.input.pop();
                    None
                }
                HintInputAct::BufferPush(c) => {
                    hs.input.push(c);
                    None
                }
                HintInputAct::Ignore => None,
            };
            if let Some(result) = result {
                hs.apply_turn(&result);
                if result.quit {
                    end_session(state);
                }
            }
        }
    }
    KeyOutcome::Handled
}

// ── Mouse ────────────────────────────────────────────────────────────────────

/// What a mouse event over the tab means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintMouse {
    /// Consumed (focus moved, scrolled, or swallowed).
    Handled,
    /// Run a registry command through the ordinary slash pipeline.
    Command(&'static str),
}

/// Route a mouse event over the Hints tab, or `None` when it is not over it.
///
/// A left click INSIDE the running hint window (transcript or input row) gives the
/// session the keyboard — and nothing else does: the tab label, Alt+4 and
/// `open-hints` only show the tab. The wheel scrolls the transcript whoever holds
/// the keyboard. The tab claims every event inside its body.
pub fn on_mouse(state: &mut AppState, m: &MouseEvent) -> Option<HintMouse> {
    if !state.hints_tab_visible() || state.any_modal_overlay_open() {
        return None;
    }
    let hits = state.hints_tab.hits();
    let pt = ratatui::layout::Position { x: m.column, y: m.row };
    if !hits.area.contains(pt) {
        return None;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hits.download.is_some_and(|r| r.contains(pt)) {
                return Some(HintMouse::Command(CMD_DOWNLOAD));
            }
            if state.overlays.hints.is_some() && state.hints_tab.phase == Phase::Running {
                state.focus = Focus::Hints;
            }
            Some(HintMouse::Handled)
        }
        _ => {
            if let Some(d) = crate::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
                let anim = state.config.animation.clone();
                let max = hits.max_scroll;
                if let Some(hs) = state.overlays.hints.as_mut() {
                    // Wheel up (d < 0) → older content, like the story transcript.
                    hs.scroll_by(if d < 0 { 1 } else { -1 }, max, &anim);
                }
            }
            Some(HintMouse::Handled)
        }
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

struct Styles {
    transcript: Style,
    input: Style,
    unfocused: Style,
    builtin: Style,
    nohint: Style,
    button: Style,
}

impl Styles {
    fn of(state: &AppState) -> Styles {
        let t = |name: &str| state.colors.theme.get(name).style;
        Styles {
            transcript: t("journal.hints.transcript"),
            input: t("journal.hints.input"),
            unfocused: t("journal.hints.input:unfocused"),
            builtin: t("journal.hints.builtin"),
            nohint: t("journal.hints.nohint"),
            button: t("journal.hints.button"),
        }
    }
}

fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// Draw the tab into `area` (the Journal body) and record where things landed.
pub fn draw(state: &AppState, area: Rect, buf: &mut Buffer) {
    let st = Styles::of(state);
    let tab = &state.hints_tab;
    let mut hits = HintsHits { area, ..HintsHits::default() };
    if area.width == 0 || area.height == 0 {
        *tab.hits.borrow_mut() = hits;
        tab.max_scroll.set(0);
        return;
    }
    fill(buf, area, st.transcript);
    match (&tab.phase, state.overlays.hints.as_ref()) {
        (Phase::Running, Some(hs)) => draw_session(state, hs, &st, area, buf, &mut hits),
        (Phase::Failed(why), _) => draw_notice(state, why, &st, area, buf, &mut hits),
        (Phase::Ended, _) => draw_notice(
            state,
            "The hint session has ended. Show this tab again to start it afresh.",
            &st,
            area,
            buf,
            &mut hits,
        ),
        _ => draw_notice(state, NO_HINT_MESSAGE, &st, area, buf, &mut hits),
    }
    tab.max_scroll.set(hits.max_scroll);
    *tab.hits.borrow_mut() = hits;
}

/// The no-hint (and failure) body: the message, the download button, and what the
/// button last did.
fn draw_notice(state: &AppState, text: &str, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut HintsHits) {
    let x = area.x + 1;
    let w = area.width.saturating_sub(2).max(1) as usize;
    let mut y = area.y;
    for line in wrap_line(text, w as u16) {
        if y >= area.bottom() {
            return;
        }
        crate::render::draw_str_clipped(buf, x, y, &line, st.nohint, area);
        y += 1;
    }
    y += 1;
    if y < area.bottom() {
        let r = Rect::new(x, y, (crate::textwidth::str_cells(DOWNLOAD_LABEL) as u16).min(area.right().saturating_sub(x)), 1);
        crate::render::draw_str_clipped(buf, x, y, DOWNLOAD_LABEL, st.button, area);
        hits.download = Some(r);
        y += 2;
    }
    if let Some(msg) = &state.hints_tab.message {
        for line in wrap_line(msg, w as u16) {
            if y >= area.bottom() {
                return;
            }
            crate::render::draw_str_clipped(buf, x, y, &line, st.nohint, area);
            y += 1;
        }
    }
}

/// The running session: companion menu (upper window), the built-in-HINT line,
/// the transcript, and the input row at the bottom.
fn draw_session(state: &AppState, session: &HintSession, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut HintsHits) {
    // The companion VM's screen model, pulled once for this frame: its upper
    // (grid) window is the InvisiClues split-screen menu drawn above the clue
    // text, and whether it awaits a keypress decides the input prompt.
    let HintSource::Zcode(vm) = &session.source;
    let char_mode = matches!(vm.pending_input(), crate::session::InputKind::Char);
    let honor = state.config.honor_game_colours;
    let companion = crate::session::screen_model_from_machine(&vm.machine);
    let focused = state.hints_have_keyboard();

    // The last row is always the input row.
    let input_y = area.bottom() - 1;
    let input_rect = Rect::new(area.x, input_y, area.width, 1);
    hits.input = input_rect;
    let input_style = if focused { st.input } else { st.unfocused };
    fill(buf, input_rect, input_style);
    let line = if char_mode {
        if focused {
            "(press a key \u{00b7} \u{2191}/\u{2193}/Enter navigate \u{00b7} Esc back to the story)".to_string()
        } else {
            "(click here, or Tab, to use the hint menu)".to_string()
        }
    } else {
        format!("> {}", session.input)
    };
    let line_cells = crate::textwidth::str_cells(&line);
    crate::render::draw_str_clipped(buf, area.x, input_y, &line, input_style, input_rect);
    if focused {
        // The cursor shows only here, in the focused input.
        let cx = area.x + line_cells as u16;
        if cx < area.right() {
            if let Some(cell) = buf.cell_mut((cx, input_y)) {
                cell.set_style(input_style.add_modifier(Modifier::REVERSED));
            }
        }
    }

    if area.height < 2 {
        return;
    }
    let mut transcript_area = Rect::new(area.x, area.y, area.width, area.height - 1);

    // Companion upper (grid) menu window: at the top, shrinking the transcript by
    // the rows it takes. Text-only hint files have no grid. The cap keeps the input
    // row plus at least two transcript rows alive under a tall menu.
    if let Some(grid) = companion.grid() {
        if grid.active_rows > 0 && transcript_area.height > 2 {
            let upper_cap = transcript_area.height - 2;
            let upper_rect = Rect::new(transcript_area.x, transcript_area.y, transcript_area.width, upper_cap);
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

    // The built-in-HINT suggestion takes the first row under the menu.
    let hint_rows: u16 = u16::from(session.builtin_hint);
    if session.builtin_hint && transcript_area.height >= 1 {
        crate::render::draw_str_clipped(
            buf, transcript_area.x, transcript_area.y, BUILTIN_LINE, st.builtin, transcript_area,
        );
    }
    if transcript_area.height <= hint_rows {
        return;
    }
    let body_top = transcript_area.y + hint_rows;
    let body_h = transcript_area.bottom() - body_top;
    let body_area = Rect::new(transcript_area.x, body_top, transcript_area.width, body_h);

    // Word-wrap each logical transcript line to the width, then show the window of
    // `body_h` rows honouring `session.scroll` (the eased value while animating).
    let wrapped: Vec<String> = session
        .transcript
        .iter()
        .flat_map(|line| wrap_line(line, body_area.width))
        .collect();
    let n = wrapped.len();
    let rows = body_h as usize;
    let max_scroll = n.saturating_sub(rows).min(u16::MAX as usize) as u16;
    hits.max_scroll = max_scroll;
    let scroll = (session.effective_scroll() as usize).min(max_scroll as usize);

    // A 1-col gutter for the scrollbar when the transcript overflows.
    let scrollbar_visible = crate::render::scroll::needs_scrollbar(n, rows) && body_area.width >= 2;
    let text_w = if scrollbar_visible { body_area.width.saturating_sub(1) } else { body_area.width };
    let text_area = Rect::new(body_area.x, body_area.y, text_w, body_area.height);

    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    for (i, line) in wrapped[start..end].iter().enumerate() {
        let row_y = body_top + i as u16;
        if row_y >= text_area.bottom() {
            break;
        }
        crate::render::draw_str_clipped(buf, text_area.x, row_y, line, st.transcript, text_area);
    }
    if scrollbar_visible {
        let sb_area = Rect::new(body_area.right().saturating_sub(1), body_area.y, 1, body_area.height);
        let look = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme);
        crate::render::scroll::draw_scrollbar(buf, sb_area, n, rows, start, look);
    }
}

/// Dim the story's input row while the hint session holds the keyboard, so which
/// of the two inputs is live is never in doubt. `story` is the story pane's rect;
/// the row is wherever the transcript draw last put the prompt.
pub fn dim_story_input(state: &AppState, story: Rect, buf: &mut Buffer) {
    if !state.hints_have_keyboard() {
        return;
    }
    let Some((_, y)) = state.input_text_origin.get() else { return };
    if y < story.y || y >= story.bottom() {
        return;
    }
    let style = state.colors.theme.get("journal.hints.input:unfocused").style;
    for x in story.x..story.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_style(style);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a minimal `HintSession` backed by the minizork.z3 fixture (a line-mode
    /// game standing in for a hint file). `None` when the fixture is absent, and the
    /// case skips.
    fn make_hint_session() -> Option<HintSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/stories/minizork-r34-s871124.z3");
        if !fixture_path.exists() {
            return None;
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec!["pick a topic".to_string()],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: "3".to_string(),
            label: "Hints: X".to_string(),
            builtin_hint: true,
        })
    }

    /// A state showing the Hints tab with `session` running.
    fn tab_state(session: HintSession) -> AppState {
        let mut state = AppState::default();
        state.journal_tab = crate::journal::JournalTab::Hints;
        state.overlays.hints = Some(session);
        state.hints_tab.phase = Phase::Running;
        state
    }

    fn screen_rows(w: u16, h: u16, state: &AppState) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(state, f.area(), f.buffer_mut())).unwrap();
        let buf = terminal.backend().buffer();
        let w = buf.area.width as usize;
        let mut rows = vec![String::new(); buf.area.height as usize];
        for (i, cell) in buf.content().iter().enumerate() {
            rows[i / w].push_str(cell.symbol());
        }
        rows
    }

    #[test]
    fn tab_renders_transcript_suggestion_and_input() {
        let Some(hs) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        let state = tab_state(hs);
        let all = screen_rows(80, 30, &state).join("\n");
        assert!(all.contains("pick a topic"), "transcript text must appear");
        assert!(all.contains("HINT"), "built-in hint suggestion ('type HINT') must appear");
        assert!(all.contains("> 3"), "the input row shows the buffer");
        assert!(state.hints_tab.hits().input.height == 1, "the input row's rect is recorded");
    }

    /// The companion VM's upper (grid) window — the InvisiClues split-screen menu —
    /// must render above the lower clue transcript.
    #[test]
    fn tab_draws_companion_upper_window_above_transcript() {
        let Some(mut hs) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        let HintSource::Zcode(vm) = &mut hs.source;
        vm.machine.screen.upper.resize(2, 8);
        vm.machine.screen.upper.put(1, 1, 'Z', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        vm.machine.screen.upper.put(1, 2, 'Q', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        vm.machine.screen.upper_window_rows = 2;
        let state = tab_state(hs);
        let rows = screen_rows(80, 30, &state);
        let upper_row = rows.iter().position(|r| r.contains("ZQ")).expect("the menu 'ZQ' must render");
        let text_row = rows.iter().position(|r| r.contains("pick a topic")).expect("transcript must render");
        assert!(upper_row < text_row, "menu (row {upper_row}) must be above the transcript (row {text_row})");
    }

    #[test]
    fn a_tab_with_no_session_draws_the_no_hint_body_not_a_panic() {
        let state = AppState::default();
        let all = screen_rows(60, 12, &state).join("\n");
        assert!(all.contains("no hint file found"), "{all}");
        assert!(all.contains("Download hints"), "{all}");
        assert!(state.hints_tab.hits().download.is_some());
    }

    #[test]
    fn a_tiny_area_draws_without_panicking() {
        let Some(hs) = make_hint_session() else { return };
        let state = tab_state(hs);
        for (w, h) in [(1, 1), (3, 1), (20, 2), (10, 3)] {
            let _ = screen_rows(w, h, &state);
        }
    }

    // ── key routing (ported from the retired modal) ───────────────────────────

    #[test]
    fn hint_keys_leave_on_esc_else_route() {
        assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Leave));
        assert!(matches!(hint_key_routes(KeyCode::Char('a')), HintKeyKind::ToSession));
    }

    /// Regression: Enter must route to the hint session input (ToSession).
    #[test]
    fn hints_enter_submits_input_not_leave() {
        assert!(matches!(hint_key_routes(KeyCode::Enter), HintKeyKind::ToSession));
    }

    /// Only PageUp/PageDown scroll the clue window; the arrow keys (and everything
    /// else) route to the companion VM so its upper-window menu stays navigable.
    #[test]
    fn hint_key_routes_pagekeys_scroll_arrows_go_to_session() {
        assert!(matches!(hint_key_routes(KeyCode::PageUp), HintKeyKind::Scroll(d) if d > 0));
        assert!(matches!(hint_key_routes(KeyCode::PageDown), HintKeyKind::Scroll(d) if d < 0));
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
                     KeyCode::Home, KeyCode::End, KeyCode::Char('h'), KeyCode::Enter] {
            assert!(matches!(hint_key_routes(code), HintKeyKind::ToSession), "{code:?} must reach the companion VM");
        }
        assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Leave));
    }

    /// In Char mode (an InvisiClues `read_char` menu) every key that reaches the
    /// session is forwarded to the companion VM, never buffered.
    #[test]
    fn hint_input_action_char_mode_forwards_keys() {
        use crate::session::InputKind::Char;
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right,
                     KeyCode::Enter, KeyCode::Backspace, KeyCode::Char('h'), KeyCode::F(1)] {
            assert_eq!(hint_input_action(Char, code), HintInputAct::ForwardKey, "char mode must forward {code:?}");
        }
    }

    #[test]
    fn hint_input_action_line_mode_edits_buffer() {
        use crate::session::InputKind::Line;
        assert_eq!(hint_input_action(Line, KeyCode::Enter), HintInputAct::SubmitLine);
        assert_eq!(hint_input_action(Line, KeyCode::Backspace), HintInputAct::BufferPop);
        assert_eq!(hint_input_action(Line, KeyCode::Char('x')), HintInputAct::BufferPush('x'));
    }

    /// End-to-end on a booted companion (which boots to `Line` mode): a printable
    /// key reaching the tab through `on_key` grows `hs.input`.
    #[test]
    fn hint_line_mode_char_buffers_into_input() {
        let Some(mut hs) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        hs.input.clear();
        {
            let HintSource::Zcode(vm) = &hs.source;
            assert_eq!(vm.pending_input(), crate::session::InputKind::Line, "companion boots to a line read");
        }
        let mut state = tab_state(hs);
        state.focus = Focus::Hints;
        let k = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(on_key(&mut state, k), KeyOutcome::Handled);
        assert_eq!(state.overlays.hints.as_ref().unwrap().input, "q", "line-mode key buffers into hs.input");
    }

    #[test]
    fn on_key_passes_everything_through_without_the_keyboard() {
        let Some(hs) = make_hint_session() else { return };
        let mut state = tab_state(hs);
        assert_eq!(state.focus, Focus::Game);
        let k = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(on_key(&mut state, k), KeyOutcome::PassThrough);
        assert_eq!(state.overlays.hints.as_ref().unwrap().input, "3", "the hint input is untouched");
    }

    /// The story's prompt row dims while the hint session has the keyboard, and only
    /// that row — in both `honor_game_colours` modes.
    #[test]
    fn the_storys_prompt_row_dims_while_the_hint_session_has_the_keyboard() {
        for honor in [true, false] {
            let Some(hs) = make_hint_session() else { return };
            let mut state = tab_state(hs);
            state.config.honor_game_colours = honor;
            let story = Rect::new(0, 0, 60, 30);
            state.input_text_origin.set(Some((4, 20)));
            let mut buf = Buffer::empty(Rect::new(0, 0, 120, 40));

            dim_story_input(&state, story, &mut buf);
            assert!(!buf.cell((10, 20)).unwrap().modifier.contains(Modifier::DIM), "story focus: not dimmed");

            state.focus = Focus::Hints;
            dim_story_input(&state, story, &mut buf);
            assert!(buf.cell((10, 20)).unwrap().modifier.contains(Modifier::DIM), "honor={honor}: prompt row dimmed");
            assert!(!buf.cell((10, 19)).unwrap().modifier.contains(Modifier::DIM), "only that row");
            assert!(!buf.cell((70, 20)).unwrap().modifier.contains(Modifier::DIM), "only inside the story pane");
        }
    }
}
