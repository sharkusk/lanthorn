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
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::engine::Engine;
use crate::hint_download::{HintDlOutcome, HintDlResult, HintDownloader};
use crate::hints::HintStory;
use crate::host::hints::{already_running, game_hint_status_with_tuid, no_hint_message, start_with_tuid, GameHintStatus, HintStart};
use crate::render::transcript::wrap_line;
use crate::state::{AppState, Focus, HintSession, HintSource};

/// The registry command the tab's button runs.
pub const CMD_DOWNLOAD: &str = "download-hints";
const DOWNLOAD_LABEL: &str = " [ Download hints… ] ";
const BUILTIN_LINE: &str = "This game has its own hints \u{2014} type HINT in the story.";
/// Lines scrolled per PageUp/PageDown while the session has the keyboard.
const HINT_PAGE_LINES: i32 = 10;

/// The dictionary the built-in-HINT check reads: empty when the story's `HINT`
/// only advertises Infocom's InvisiClues booklets (SQ-1745).
fn builtin_hint_words(state: &AppState) -> &[String] {
    if state.hint_booklet_notice {
        &[]
    } else {
        &state.dict_words
    }
}

/// Where the tab's session stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    /// Not tried yet: the next time the tab is on screen, [`ensure_started`] opens it.
    #[default]
    NotStarted,
    /// `overlays.hints` holds the running session.
    Running,
    /// Resolution found no hint file.
    NoHint,
    /// Several hint files tie and the player picks one (SQ-1690): listed as rows,
    /// clicked or chosen with Up/Down/Enter once the tab has the keyboard. The pick
    /// is remembered per IFID.
    Choose(Vec<PathBuf>),
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
    /// The chooser's rows, in candidate order (SQ-1690).
    pub choices: Vec<Rect>,
    /// Largest transcript scroll offset of the last draw.
    pub max_scroll: u16,
}

/// The tab's own state; the session is `AppState.overlays.hints`.
pub struct HintsTab {
    pub phase: Phase,
    /// What the button or a download last did, shown under the no-hint message.
    pub message: Option<String>,
    /// The game's documents folder as of the last start, for the no-hint message
    /// (`None`: not linked to IFDB).
    pub docs: Option<PathBuf>,
    /// The chooser's highlighted row.
    pub choice: usize,
    /// Where the game's hints stood at the last start; the tab offers the download
    /// button exactly when this is [`GameHintStatus::Downloadable`].
    pub status: GameHintStatus,
    /// A hint file the player picked, to remember and open at the next start.
    picked: Option<PathBuf>,
    downloader: HintDownloader,
    hits: RefCell<HintsHits>,
    max_scroll: Cell<u16>,
}

impl Default for HintsTab {
    fn default() -> Self {
        HintsTab {
            phase: Phase::NotStarted,
            message: None,
            docs: None,
            choice: 0,
            status: GameHintStatus::None,
            picked: None,
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
    /// Replace the downloader, so a host or test can supply its own fetcher
    /// ([`HintDownloader::with_fetcher`]).
    pub fn set_downloader(&mut self, downloader: HintDownloader) {
        self.downloader = downloader;
    }

    /// Where the last draw put things.
    /// The `(rows, cols)` the hint program should be laid out at: the panel the last
    /// draw gave it, less the input row. `None` before the tab has been drawn at all
    /// (SQ-1753).
    pub fn panel_screen(&self) -> Option<(u16, u16)> {
        let area = self.hits.borrow().area;
        (area.width > 0 && area.height > 1).then(|| (area.height - 1, area.width))
    }

    pub fn hits(&self) -> HintsHits {
        self.hits.borrow().clone()
    }
}

/// The TUI's key help after the chooser's prompt ([`crate::host::hints::CHOOSE_PROMPT`]).
const CHOOSE_KEYS: &str = " (click it, or Up/Down and Enter once the tab has the keyboard):";

/// The chooser's intro line, shown above the tied candidates (SQ-1694).
pub fn choose_intro() -> String {
    format!("{}{CHOOSE_KEYS}", crate::host::hints::CHOOSE_PROMPT)
}

// ── Starting, ending, downloading ────────────────────────────────────────────

/// The running game's documents folder, existing or planned; `None` when the game
/// is not linked to IFDB or the host has no library (SQ-1690). Loads the story's
/// record, so callers ask only when they are about to start or download.
pub fn game_documents_dir(state: &AppState, story_path: &Path) -> Option<PathBuf> {
    let roots = state.data_roots.as_ref()?;
    let entry = crate::picker::resolve_entry_from(story_path, state.source.disk_entry.as_deref(), roots)?;
    crate::documents::locate(roots, entry.meta.ifdb_tuid.as_deref(), &entry.title).path().map(Path::to_path_buf)
}

/// The running game's IFDB tuid, from its library record; `None` when the game is
/// not linked or the host has no library. The identity fallback for an IFID the
/// registry does not know ([`crate::hints::hint_download_for_with_tuid`]).
pub fn game_ifdb_tuid(state: &AppState, story_path: &Path) -> Option<String> {
    let roots = state.data_roots.as_ref()?;
    crate::picker::resolve_entry_from(story_path, state.source.disk_entry.as_deref(), roots)?.meta.ifdb_tuid
}

/// Start the session the first time the tab is on screen. Called once per loop
/// turn; `true` when something changed and a redraw is due.
///
/// Resolution is [`crate::host::hints::open`] unchanged — this only decides WHEN,
/// and asks [`crate::host::hints::available`] first so that a tie becomes the
/// chooser ([`Phase::Choose`]) instead of "no hint file".
pub fn ensure_started(state: &mut AppState, story_path: &Path, ifid: &str) -> bool {
    let tab = &state.hints_tab;
    if !state.hints_tab_visible() || (tab.phase != Phase::NotStarted && tab.picked.is_none()) {
        return false;
    }
    let documents = game_documents_dir(state, story_path);
    let tuid = game_ifdb_tuid(state, story_path);
    ensure_started_in_with_tuid(state, story_path, ifid, tuid.as_deref(), documents.as_deref())
}

/// [`ensure_started`] with the game's documents folder already known (the seam the
/// tests use; `documents` is what [`game_documents_dir`] answers).
pub fn ensure_started_in(state: &mut AppState, story_path: &Path, ifid: &str, documents: Option<&Path>) -> bool {
    ensure_started_in_with_tuid(state, story_path, ifid, None, documents)
}

/// [`ensure_started_in`] with the game's IFDB tuid (see [`game_ifdb_tuid`]).
pub fn ensure_started_in_with_tuid(
    state: &mut AppState,
    story_path: &Path,
    ifid: &str,
    tuid: Option<&str>,
    documents: Option<&Path>,
) -> bool {
    if !state.hints_tab_visible() {
        return false;
    }
    let picked = state.hints_tab.picked.take();
    if picked.is_none() && state.hints_tab.phase != Phase::NotStarted {
        return false;
    }
    let title = state.title.clone();
    let story = HintStory::new(ifid, &title).with_documents(documents);
    state.hints_tab.docs = documents.map(Path::to_path_buf);
    let running = if state.hints_tab.phase == Phase::Running {
        state.overlays.hints.as_ref().map(|hs| hs.label.clone())
    } else {
        None
    };
    state.hints_tab.phase = Phase::NotStarted;
    let index = crate::hints::load_hint_index(&state.config.user_dir);
    state.hints_tab.status = game_hint_status_with_tuid(story_path, story, tuid, &index);
    match start_with_tuid(story_path, story, tuid, picked.as_deref(), running.as_deref(), builtin_hint_words(state), &state.config, state.hints_tab.panel_screen()) {
        HintStart::Started(session) => {
            state.overlays.hints = Some(*session);
            state.hints_tab.phase = Phase::Running;
        }
        HintStart::AlreadyRunning => state.hints_tab.phase = Phase::Running,
        HintStart::Choose(candidates) => {
            state.hints_tab.choice = 0;
            state.hints_tab.phase = Phase::Choose(candidates);
        }
        HintStart::NoHint(_) => state.hints_tab.phase = Phase::NoHint,
        HintStart::Failed(e) => state.hints_tab.phase = Phase::Failed(e.to_string()),
    }
    true
}

/// Tell a running hint program the panel's size when the panel has been resized since
/// it booted, the way the story pane's `sync_zvm_screen_dims` does for the story (SQ-1753).
/// Returns `true` when the program's header changed. The boot size is what the program
/// laid itself out at (see [`crate::host::hints::open`]); this keeps its later menu
/// redraws and wrapping in step.
pub fn sync_screen_dims(state: &mut AppState) -> bool {
    use crate::engine::Engine;
    let Some((rows, cols)) = state.hints_tab.panel_screen() else { return false };
    let Some(HintSession { source: HintSource::Zcode(vm), .. }) = state.overlays.hints.as_mut() else {
        return false;
    };
    let version = vm.machine.mem.version();
    if version < 4 || version == 6 {
        return false;
    }
    let current = (vm.machine.mem.read_byte(0x20) as u16, vm.machine.mem.read_byte(0x21) as u16);
    if current == (rows.min(255), cols.min(255)) {
        return false;
    }
    vm.set_screen_dims(rows, cols);
    true
}

/// A hint program was opened from the Documents tab (SQ-1690): show the Hints tab
/// running THAT file, starting (or restarting) the session as needed. Like the tab
/// click and Alt+4 it does not move the keyboard; the loop's next turn starts the
/// session, remembering the file as this game's hint file.
pub fn show_program(state: &mut AppState, path: PathBuf) {
    let already = state.hints_tab.phase == Phase::Running
        && already_running(state.overlays.hints.as_ref().map(|hs| hs.label.as_str()), &path);
    if !already {
        state.overlays.hints = None;
        state.hints_tab.phase = Phase::NotStarted;
        state.hints_tab.picked = Some(path);
    }
    state.set_journal_tab(crate::journal::JournalTab::Hints);
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

/// `download-hints` for the running game: fetch a matching InvisiClues file into
/// its documents folder when it is linked to IFDB, else beside the story. The
/// result arrives through [`poll_download`].
pub fn start_download(state: &mut AppState, story_path: &Path) {
    let documents = game_documents_dir(state, story_path);
    let tuid = game_ifdb_tuid(state, story_path);
    start_download_in_with_tuid(state, story_path, tuid.as_deref(), documents);
}

/// [`start_download`] with the documents folder already known.
pub fn start_download_in(state: &mut AppState, story_path: &Path, documents: Option<PathBuf>) {
    start_download_in_with_tuid(state, story_path, None, documents);
}

/// [`start_download_in`] with the game's IFDB tuid (see [`game_ifdb_tuid`]).
pub fn start_download_in_with_tuid(
    state: &mut AppState,
    story_path: &Path,
    tuid: Option<&str>,
    documents: Option<PathBuf>,
) {
    let running = state.hints_tab.phase == Phase::Running;
    let line = crate::host::hints::start_game_download_with_tuid(
        &mut state.hints_tab.downloader,
        &state.ifid,
        tuid,
        &state.title,
        story_path,
        state.source.disk_entry.as_deref(),
        documents.as_deref(),
        running,
    );
    state.hints_tab.message = Some(line.clone());
    state.set_status(line);
}

/// Drain finished downloads and hand them back, so a host can fold each into its
/// story list ([`crate::picker::apply_hint_download`]). A completed one wrote a
/// hint file (into the documents folder or beside the story), so the tab asks to
/// resolve again; each result's line becomes the tab's message and the status.
pub fn drain_downloads(state: &mut AppState) -> Vec<HintDlResult> {
    let results = state.hints_tab.downloader.drain();
    for r in &results {
        if r.outcome == HintDlOutcome::Done {
            state.hints_tab.phase = Phase::NotStarted;
        }
        let line = crate::hint_download::download_result_line(r);
        state.hints_tab.message = Some(line.clone());
        state.set_status(line);
    }
    results
}

/// [`drain_downloads`] for the run loop: `true` when a redraw is due.
pub fn poll_download(state: &mut AppState) -> bool {
    let changed = !drain_downloads(state).is_empty();
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
    if let Phase::Choose(candidates) = &state.hints_tab.phase {
        let last = candidates.len().saturating_sub(1);
        let current = state.hints_tab.choice.min(last);
        match key.code {
            KeyCode::Up => state.hints_tab.choice = current.saturating_sub(1),
            KeyCode::Down => state.hints_tab.choice = (current + 1).min(last),
            KeyCode::Enter => state.hints_tab.picked = candidates.get(current).cloned(),
            KeyCode::Esc => state.focus = Focus::Game,
            KeyCode::Tab | KeyCode::BackTab => state.cycle_focus(key.code == KeyCode::Tab),
            _ => {}
        }
        return KeyOutcome::Handled;
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
            if let Phase::Choose(candidates) = &state.hints_tab.phase {
                if let Some(i) = hits.choices.iter().position(|r| r.contains(pt)) {
                    state.hints_tab.choice = i;
                    state.hints_tab.picked = candidates.get(i).cloned();
                }
                state.focus = Focus::Hints;
            } else if state.overlays.hints.is_some() && state.hints_tab.phase == Phase::Running {
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
pub fn draw(state: &AppState, outer: Rect, buf: &mut Buffer) {
    let st = Styles::of(state);
    // The frame (shared with the Inventory tab) first; everything below, and
    // every hit-rect, lives in its inner rect.
    let area = if outer.width == 0 || outer.height == 0 {
        outer
    } else {
        crate::journal::frame_body(buf, outer, "Hints", &state.colors, state.hints_have_keyboard())
    };
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
        (Phase::Choose(candidates), _) => draw_choose(state, candidates, &st, area, buf, &mut hits),
        _ => draw_notice(state, &no_hint_message(tab.docs.as_deref()), &st, area, buf, &mut hits),
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
    if state.hints_tab.status == GameHintStatus::Downloadable && y < area.bottom() {
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

/// The chooser (SQ-1690): the tied candidates as rows under one explaining line.
fn draw_choose(
    state: &AppState,
    candidates: &[PathBuf],
    st: &Styles,
    area: Rect,
    buf: &mut Buffer,
    hits: &mut HintsHits,
) {
    let x = area.x + 1;
    let w = area.width.saturating_sub(2).max(1) as usize;
    let mut y = area.y;
    for line in wrap_line(&choose_intro(), w as u16) {
        if y >= area.bottom() {
            return;
        }
        crate::render::draw_str_clipped(buf, x, y, &line, st.nohint, area);
        y += 1;
    }
    y += 1;
    let focused = state.hints_have_keyboard();
    for (i, path) in candidates.iter().enumerate() {
        if y >= area.bottom() {
            break;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let selected = i == state.hints_tab.choice;
        let label = format!(" {} {} ", if selected { '\u{25b8}' } else { ' ' }, name);
        let style = if selected && focused { st.button.add_modifier(Modifier::REVERSED) } else { st.button };
        let width = (crate::textwidth::str_cells(&label) as u16).min(area.right().saturating_sub(x));
        crate::render::draw_str_clipped(buf, x, y, &label, style, area);
        hits.choices.push(Rect::new(x, y, width, 1));
        y += 1;
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
    let show_builtin = session.shows_builtin_line();
    let hint_rows: u16 = u16::from(show_builtin);
    if show_builtin && transcript_area.height >= 1 {
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

    /// SQ-1753: the panel's drawn size is what a running hint program is told, and a later
    /// resize of the panel is re-declared; before the first draw there is no size to give.
    #[test]
    fn a_running_hint_program_follows_the_panel_size() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stories/Tangle.z5");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: Tangle.z5 fixture absent");
            return;
        };
        let vm = crate::session::GameSession::new(bytes, true, false, None).expect("v5 session");
        let hs = HintSession {
            source: HintSource::Zcode(vm),
            transcript: Vec::new(),
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: X".to_string(),
            builtin_hint: false,
        };
        let mut state = tab_state(hs);
        assert_eq!(state.hints_tab.panel_screen(), None, "nothing drawn yet, so no size to give");
        assert!(!sync_screen_dims(&mut state));

        let _ = screen_rows(60, 20, &state);
        assert_eq!(state.hints_tab.panel_screen(), Some((17, 58)), "the framed panel less its input row");
        assert!(sync_screen_dims(&mut state), "the header changes to the panel's size");
        let HintSource::Zcode(vm) = &state.overlays.hints.as_ref().unwrap().source;
        assert_eq!((vm.machine.mem.read_byte(0x20), vm.machine.mem.read_byte(0x21)), (17, 58));
        assert!(!sync_screen_dims(&mut state), "a second call has nothing to change");

        let _ = screen_rows(50, 14, &state);
        assert!(sync_screen_dims(&mut state), "a later resize is re-declared");
        let HintSource::Zcode(vm) = &state.overlays.hints.as_ref().unwrap().source;
        assert_eq!((vm.machine.mem.read_byte(0x20), vm.machine.mem.read_byte(0x21)), (11, 48));
    }

    #[test]
    fn a_booklet_only_hint_word_does_not_count_as_builtin_hints() {
        let mut state = AppState::default();
        state.dict_words = vec!["hint".to_string(), "look".to_string()];
        assert!(crate::hints::story_supports_hint(builtin_hint_words(&state).iter().cloned()));
        state.hint_booklet_notice = true;
        assert!(!crate::hints::story_supports_hint(builtin_hint_words(&state).iter().cloned()));
    }

    #[test]
    fn tab_renders_transcript_and_input_without_the_builtin_suggestion_while_a_companion_runs() {
        let Some(hs) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        assert!(hs.builtin_hint, "non-vacuity: the session was told the story has a HINT word");
        assert!(!hs.shows_builtin_line());
        let state = tab_state(hs);
        let all = screen_rows(80, 30, &state).join("\n");
        assert!(all.contains("pick a topic"), "transcript text must appear");
        assert!(!all.contains("type HINT"), "SQ-1745: no 'type HINT' line above a running companion");
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
        let mut state = AppState::default();
        state.hints_tab.status = GameHintStatus::Downloadable;
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

    // ── the frame (shared with the Inventory tab) ─────────────────────────────

    fn frameless(state: &mut AppState) {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed = crate::theme::toml_schema::parse("[panel]\nborder = { style = \"none\" }\n").unwrap();
        state.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
    }

    #[test]
    fn the_body_is_framed_with_the_tab_name_and_the_download_button_sits_inside_it() {
        let mut state = AppState::default();
        state.hints_tab.status = GameHintStatus::Downloadable;
        let rows = screen_rows(60, 12, &state);
        let cells: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
        assert_eq!((cells[0][0], cells[0][59]), ('\u{250c}', '\u{2510}'), "top corners: {:?}", rows[0]);
        assert_eq!((cells[11][0], cells[11][59]), ('\u{2514}', '\u{2518}'), "bottom corners");
        assert_eq!(cells[5][0], '\u{2502}', "left edge");
        assert!(rows[0].contains("Hints"), "tab name as the title: {:?}", rows[0]);
        let dl = state.hints_tab.hits().download.expect("download button");
        assert!(dl.x >= 1 && dl.y >= 1 && dl.right() <= 59 && dl.bottom() <= 11, "inside the frame: {dl:?}");
        let mut state = state;
        state.journal_tab = crate::journal::JournalTab::Hints;
        let click = crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: dl.x + 1,
            row: dl.y,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        assert!(matches!(on_mouse(&mut state, &click), Some(HintMouse::Command(CMD_DOWNLOAD))));
    }

    #[test]
    fn the_input_row_is_inside_the_frame() {
        let Some(hs) = make_hint_session() else { return };
        let state = tab_state(hs);
        let rows = screen_rows(60, 12, &state);
        let input = state.hints_tab.hits().input;
        assert!(input.x >= 1 && input.right() <= 59 && input.bottom() <= 11, "{input:?}");
        assert!(rows[11].starts_with('\u{2514}'), "bottom border below the input row");
    }

    #[test]
    fn frameless_drops_the_frame_like_the_other_tabs() {
        let mut state = AppState::default();
        state.hints_tab.status = GameHintStatus::Downloadable;
        frameless(&mut state);
        let rows = screen_rows(60, 12, &state);
        // Like the Inventory tab: no border glyphs, the title becomes a plain header row.
        assert!(rows.iter().all(|r| !r.contains('\u{250c}') && !r.contains('\u{2502}')), "{rows:?}");
        assert!(rows[0].contains("Hints"), "{:?}", rows[0]);
        assert!(rows.iter().any(|r| r.contains("Download hints")));
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
