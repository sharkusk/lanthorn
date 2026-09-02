//! The story-picker's IFDB search modal (SQ-0413): a three-view state machine
//! (type a query → browse results → pick among a game's download files) plus
//! its renderer. All network work is delegated to
//! [`crate::ifdb_search::SearchWorker`]; this module is the pure UI half — the
//! picker drives it by feeding keys/events in and dispatching the [`ModalAction`]s
//! it hands back. Every state transition is unit-tested without a network.
//!
//! Standing modal conventions (matching the picker's other modals): Enter
//! activates, Up/Down (and j/k) navigate lists. While a request is in flight
//! the modal is "busy": keystrokes are ignored except Esc, which abandons the
//! pending result.
//!
//! SQ-0473: the modal opens showing a "Popular on IFDB" seed list instead of
//! an empty query box — see [`SearchModal::open`] and the module's Esc-ladder
//! note below.
//!
//! Esc ladder (results → query → close), with SQ-0473's one twist: Esc from
//! the seed list closes the modal outright (one level shorter than before,
//! since there's no "empty query" screen behind it worth a stop); Esc from a
//! *typed* search's results returns to the seed list, cached from modal-open,
//! rather than to an empty box; Esc while typing (query box active) still
//! closes, unchanged. See `results_key` below.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::colors::ColorScheme;
use crate::config::AnimationConfig;
use crate::ifdb_search::{DownloadOption, SearchEvent, SearchHit};
use crate::ifiction::IFiction;
use crate::list_scroll::ListScroll;
use crate::render::dialog::{
    draw_dialog, DialogField, DialogRects, DialogSpec, DialogStyle, Placement,
};
use crate::text_field::TextField;

/// Which view the modal is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// Typing a query.
    Input,
    /// Browsing search hits.
    Results,
    /// Choosing among several playable download files for one game.
    Choosing,
}

/// The kind of request currently in flight (if any). Guards against a stale
/// result being applied after the user backed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inflight {
    /// The "Popular on IFDB" seed query (SQ-0473), fetched once on open.
    Seed,
    Search,
    Resolve,
    Download,
}

/// What the picker should do in response to a key/event. The picker owns the
/// worker and the download directory, so it translates these into jobs.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalAction {
    /// Nothing to do.
    None,
    /// Close the modal.
    Close,
    /// Dispatch a game search for this query.
    Search(String),
    /// Resolve download options for this IFDB tuid.
    Resolve(String),
    /// Download this file URL (the picker supplies the destination dir).
    Download(String),
    /// Open this IFDB page URL in the browser (the "no playable file" fallback).
    OpenInBrowser(String),
    /// Dispatch the "Popular on IFDB" seed query (SQ-0473) — returned once by
    /// [`SearchModal::open`], right after construction.
    Seed,
}

/// The modal's full state.
pub struct SearchModal {
    query: TextField,
    view: View,
    inflight: Option<Inflight>,
    hits: Vec<SearchHit>,
    /// Selection + scroll offset for `hits`, shared with the story picker
    /// (SQ-0598). A stateless "recompute the window from the index each frame"
    /// rule can only ever pin the selection to one edge of the viewport — going
    /// back up, the cursor froze on the bottom row and the list scrolled under
    /// it until the top. `ListScroll` keeps the offset and moves it only when
    /// the selection would actually leave the viewport.
    hit_scroll: ListScroll,
    /// The "Popular on IFDB" seed list (SQ-0473), cached for this modal
    /// instance's lifetime once it arrives — reused when Esc backs out of a
    /// typed search rather than re-fetching. NOT persisted across separate
    /// modal opens (each `/` press builds a fresh `SearchModal`, so a second
    /// open re-fetches); see the module header.
    seed_hits: Vec<SearchHit>,
    /// True while `hits` holds the seed list rather than a typed search's
    /// results — governs the Esc ladder and the "Popular on IFDB" hint label.
    showing_seed: bool,
    options: Vec<DownloadOption>,
    /// Selection + scroll offset for `options`, in *items* rather than rows —
    /// a download row is two rows tall when any of them has a description.
    opt_scroll: ListScroll,
    /// Parallel to `options`: whether the download directory already holds a
    /// file of that name (SQ-0597). Computed once, when the options land,
    /// rather than per frame — the renderer runs on every keystroke and this
    /// is a filesystem probe.
    opt_present: Vec<bool>,
    /// Where a download would land, so `opt_present` can be filled in. `None`
    /// until the picker supplies it (and in the unit tests), which simply means
    /// nothing is reported as already downloaded.
    download_dir: Option<std::path::PathBuf>,
    /// Items (not rows) the list viewport last fitted, recorded by the renderer
    /// because only it knows the dialog's height and the current row stride.
    /// Key handling needs it to decide when a move scrolls; 1 until the first
    /// frame, which keeps a key pressed before any draw well-defined.
    list_rows: usize,
    /// The iFiction record resolved alongside `options` (SQ-0474), if any —
    /// carried from the last `SearchEvent::Options` through to the `Download`
    /// action the user's pick in `View::Choosing` produces, so the picker can
    /// populate the download's sidecar + cover with zero extra
    /// requests. Taken (not cloned) by [`Self::take_pending_record`] once
    /// the picker dispatches that download. Boxed to match
    /// [`crate::ifdb_search::ResolvedGame::record`].
    pending_record: Option<Box<IFiction>>,
    /// A transient status/error line shown under the frame title.
    status: Option<String>,
}

impl SearchModal {
    pub fn new() -> Self {
        SearchModal {
            query: TextField::new(""),
            view: View::Input,
            inflight: None,
            hits: Vec::new(),
            hit_scroll: ListScroll::new(),
            seed_hits: Vec::new(),
            showing_seed: false,
            options: Vec::new(),
            opt_scroll: ListScroll::new(),
            opt_present: Vec::new(),
            download_dir: None,
            list_rows: 1,
            pending_record: None,
            status: None,
        }
    }

    /// Kick off the "Popular on IFDB" seed query (SQ-0473). Call once, right
    /// after construction; marks the modal busy (its normal "Searching…"
    /// state) and returns the action for the picker to dispatch to the
    /// worker. A failed or empty seed just leaves the modal on its ordinary
    /// empty query box (see `on_event`) — it never blocks opening.
    pub fn open(&mut self) -> ModalAction {
        self.inflight = Some(Inflight::Seed);
        ModalAction::Seed
    }

    /// True while a request is in flight (used to keep the picker's redraw loop
    /// polling for the worker's result).
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// The title of the currently-selected hit, for the "Downloaded X" toast /
    /// progress line the picker shows on success.
    pub fn selected_hit_title(&self) -> Option<&str> {
        self.hits.get(self.hit_scroll.selected).map(|h| h.title.as_str())
    }

    /// True while either list's scroll offset is still easing, so the picker's
    /// run loop keeps ticking and the motion is actually drawn (same contract as
    /// its own `list`).
    pub fn has_active_animation(&self) -> bool {
        self.hit_scroll.has_active_animation() || self.opt_scroll.has_active_animation()
    }

    /// Drop a finished scroll tween so the settled frame paints once. Mirrors
    /// `ListScroll::finalize_if_done`; returns whether anything was cleared.
    pub fn finalize_if_done(&mut self) -> bool {
        // Both, not short-circuited — either may be mid-tween.
        let h = self.hit_scroll.finalize_if_done();
        let o = self.opt_scroll.finalize_if_done();
        h || o
    }

    /// Tell the modal where a download would land, so the chooser can mark the
    /// files that directory already holds. The picker calls this right after
    /// construction; without it nothing is marked.
    pub fn set_download_dir(&mut self, dir: &std::path::Path) {
        self.download_dir = Some(dir.to_path_buf());
    }

    /// Which of `options` the download directory already has, by the exact
    /// filename the download would be saved under.
    ///
    /// Names only — not contents. Two IFDB entries can share a filename while
    /// being different builds (both of Photopia's `photopia.z5` links), so a
    /// match means "a file of this name is already here", which is precisely
    /// what decides whether downloading again produces a `foo (2).z5`
    /// duplicate. It is a hint on the row, never a block on the download.
    fn probe_present(&self, options: &[DownloadOption]) -> Vec<bool> {
        let Some(dir) = &self.download_dir else { return vec![false; options.len()] };
        options.iter().map(|o| dir.join(&o.filename).exists()).collect()
    }

    /// Point the hit list back at its first row after `hits` is replaced. A
    /// fresh `ListScroll` rather than `selected = 0`, so the scroll *offset*
    /// resets too — otherwise a new, shorter result set would render from a
    /// window scrolled past its end.
    fn reset_hit_scroll(&mut self) {
        self.hit_scroll = ListScroll::new();
        self.hit_scroll.len(self.hits.len());
    }

    /// Consume the iFiction record resolved for the game currently being
    /// downloaded (SQ-0474). The picker calls this exactly once, when
    /// dispatching a `ModalAction::Download` to the worker, so the download
    /// job can populate the sidecar + cover — see `pending_record`.
    pub fn take_pending_record(&mut self) -> Option<Box<IFiction>> {
        self.pending_record.take()
    }

    // ── Key handling ──────────────────────────────────────────────────────

    /// Feed a keypress; returns what the picker should do. `anim` is the
    /// picker's animation config, so list scrolling eases exactly as the story
    /// list's does.
    pub fn on_key(&mut self, code: KeyCode, anim: &AnimationConfig) -> ModalAction {
        // While busy, only Esc responds — it abandons the pending result.
        if self.inflight.is_some() {
            if code == KeyCode::Esc {
                self.inflight = None;
                self.status = Some("Cancelled".to_string());
            }
            return ModalAction::None;
        }
        // A fresh keystroke clears a stale status line.
        self.status = None;
        match self.view {
            View::Input => self.input_key(code),
            View::Results => self.results_key(code, anim),
            View::Choosing => self.choosing_key(code, anim),
        }
    }

    /// Apply a list-navigation key to `scroll`, or report that it wasn't one.
    /// Shared by both list views so they navigate identically — and identically
    /// to the story picker's list view and the command band's `PageUp`/
    /// `PageDown`/`Home`/`End`, which all drive the same [`list_scroll::nav_key`]
    /// (SQ-0682: promoted here from a private helper local to this modal so it
    /// really is one mechanism, not three near-identical copies).
    fn nav_key(scroll: &mut ListScroll, code: KeyCode, total: usize, rows: usize, anim: &AnimationConfig) -> bool {
        crate::list_scroll::nav_key(scroll, code, total, rows, anim)
    }

    /// Feed a mouse-wheel notch: `delta` is ±1 row, already resolved for the
    /// `mouse_wheel_invert` preference by `input::wheel_delta` (never invert
    /// again here). Unlike a nav key, the wheel SCROLLS the visible list and
    /// clamps the selection into the window rather than moving the cursor
    /// (SQ-0831) — see `ListScroll::scroll_by`, which both list views drive.
    /// A list that fits its window has nothing to scroll, and the notch does
    /// nothing at all.
    pub fn on_wheel(&mut self, delta: isize, anim: &AnimationConfig) {
        // While a request is in flight the modal ignores input but for Esc; the
        // wheel is no exception.
        if self.inflight.is_some() {
            return;
        }
        match self.view {
            // `View::Input` still shows whatever list was there (SQ-0473's
            // type-over-the-list), so the wheel scrolls it — only the query box
            // itself is not a list.
            View::Input | View::Results => {
                let len = self.hits.len();
                self.hit_scroll.len(len);
                self.hit_scroll.scroll_by(delta, self.list_rows, anim);
            }
            View::Choosing => {
                let len = self.options.len();
                self.opt_scroll.len(len);
                self.opt_scroll.scroll_by(delta, self.list_rows, anim);
            }
        }
    }

    fn input_key(&mut self, code: KeyCode) -> ModalAction {
        match code {
            KeyCode::Esc => ModalAction::Close,
            // SQ-0691: Tab/Shift-Tab hop to the results list (when there is one) without
            // running a search, keeping the half-typed query. Typing over the list already
            // dropped you INTO the query editor; this is the way back out — the only exits
            // used to be Enter (a search you didn't want yet) or Esc (closes the modal).
            KeyCode::Tab | KeyCode::BackTab if !self.hits.is_empty() => {
                self.view = View::Results;
                ModalAction::None
            }
            KeyCode::Enter => {
                let q = self.query.as_str().trim().to_string();
                if q.is_empty() {
                    return ModalAction::None;
                }
                self.inflight = Some(Inflight::Search);
                ModalAction::Search(q)
            }
            KeyCode::Backspace => {
                self.query.backspace();
                ModalAction::None
            }
            KeyCode::Delete => {
                self.query.delete();
                ModalAction::None
            }
            KeyCode::Left => {
                self.query.left();
                ModalAction::None
            }
            KeyCode::Right => {
                self.query.right();
                ModalAction::None
            }
            KeyCode::Home => {
                self.query.home();
                ModalAction::None
            }
            KeyCode::End => {
                self.query.end();
                ModalAction::None
            }
            KeyCode::Char(c) => {
                self.query.insert(c);
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    fn results_key(&mut self, code: KeyCode, anim: &AnimationConfig) -> ModalAction {
        // Nav first: Home/End/PageUp/PageDown are not `Char`s, so they can't
        // collide with the type-to-search fallback at the bottom of this match.
        if Self::nav_key(&mut self.hit_scroll, code, self.hits.len(), self.list_rows, anim) {
            return ModalAction::None;
        }
        match code {
            // SQ-0473 Esc ladder: from the seed list, close outright (nothing
            // useful behind it); from a typed search's results, return to the
            // cached seed list if there is one, else fall back to the old
            // empty-query behaviour (no seed ever loaded/loaded empty).
            KeyCode::Esc => {
                if self.showing_seed {
                    return ModalAction::Close;
                }
                if !self.seed_hits.is_empty() {
                    self.hits = self.seed_hits.clone();
                    self.reset_hit_scroll();
                    self.showing_seed = true;
                } else {
                    self.view = View::Input;
                }
                ModalAction::None
            }
            KeyCode::Enter => match self.hits.get(self.hit_scroll.selected) {
                Some(hit) => {
                    self.inflight = Some(Inflight::Resolve);
                    ModalAction::Resolve(hit.tuid.clone())
                }
                None => ModalAction::None,
            },
            // SQ-0691: the mirror of the hop in `input_key` — focus the query field, list
            // selection and query text both intact.
            KeyCode::Tab | KeyCode::BackTab => {
                self.view = View::Input;
                ModalAction::None
            }
            // Fallback: open the game's IFDB page in a browser.
            KeyCode::Char('o') => {
                match self.hits.get(self.hit_scroll.selected).and_then(|h| h.link.clone()) {
                    Some(link) => ModalAction::OpenInBrowser(link),
                    None => ModalAction::None,
                }
            }
            // SQ-0473: typing over the list (seed or typed-search results)
            // starts a fresh query edit — the list stays visible underneath
            // (see render_body's View::Input arm) until Enter runs the real
            // search, same as typing always has. 'j'/'k'/'o' stay reserved for
            // nav/open as the *first* keystroke (matching this view's existing
            // bindings above); IFDB search is case-insensitive, so a query that
            // starts with one of those letters can still be typed by starting
            // with its capital (e.g. "Jigsaw").
            KeyCode::Char(c) => {
                self.view = View::Input;
                self.input_key(KeyCode::Char(c))
            }
            _ => ModalAction::None,
        }
    }

    fn choosing_key(&mut self, code: KeyCode, anim: &AnimationConfig) -> ModalAction {
        if Self::nav_key(&mut self.opt_scroll, code, self.options.len(), self.list_rows, anim) {
            return ModalAction::None;
        }
        match code {
            // Back to the results list.
            KeyCode::Esc => {
                self.view = View::Results;
                self.options.clear();
                self.opt_present.clear();
                self.opt_scroll = ListScroll::new();
                ModalAction::None
            }
            KeyCode::Enter => match self.options.get(self.opt_scroll.selected) {
                Some(opt) => {
                    self.inflight = Some(Inflight::Download);
                    ModalAction::Download(opt.url.clone())
                }
                None => ModalAction::None,
            },
            _ => ModalAction::None,
        }
    }

    // ── Worker events ─────────────────────────────────────────────────────

    /// Feed a worker event. Returns a follow-up action. A
    /// [`SearchEvent::Downloaded`] is
    /// NOT handled here — the picker consumes that directly (it owns the path
    /// and the list refresh) and then closes the modal.
    pub fn on_event(&mut self, ev: &SearchEvent) -> ModalAction {
        match ev {
            SearchEvent::Results(hits) => match self.inflight {
                Some(Inflight::Search) => {
                    self.inflight = None;
                    self.hits = hits.clone();
                    self.reset_hit_scroll();
                    self.showing_seed = false;
                    if hits.is_empty() {
                        self.status = Some("No games found".to_string());
                        self.view = View::Input;
                    } else {
                        self.view = View::Results;
                    }
                    ModalAction::None
                }
                Some(Inflight::Seed) => {
                    // SQ-0473: cache the seed list; an empty reply just leaves
                    // the modal on its ordinary empty query box (View::Input,
                    // already the starting state) rather than forcing a status
                    // line for what's an automatic, non-user-initiated load.
                    self.inflight = None;
                    self.seed_hits = hits.clone();
                    if !hits.is_empty() {
                        self.hits = hits.clone();
                        self.reset_hit_scroll();
                        self.showing_seed = true;
                        self.view = View::Results;
                    }
                    ModalAction::None
                }
                _ => ModalAction::None, // stale (user backed out, or another job in flight)
            },
            SearchEvent::Options(resolved) => {
                if self.inflight != Some(Inflight::Resolve) {
                    return ModalAction::None; // stale
                }
                self.inflight = None;
                // Cached for whichever `Download` action the user's pick in
                // `View::Choosing` produces — it always wants this game's
                // record (SQ-0474).
                self.pending_record = resolved.record.clone();
                if resolved.options.is_empty() {
                    self.status = Some(
                        "No directly-playable file — press o to open its IFDB page".to_string(),
                    );
                    self.view = View::Results;
                    return ModalAction::None;
                }
                // Every game with a playable file goes through the chooser,
                // including one with exactly a single file. That case used to
                // download straight away, which meant the one screen carrying a
                // file's description and its "already downloaded" mark was the
                // one screen you never saw — and a game you already had would
                // silently fetch a duplicate with nothing on screen to warn you.
                self.opt_present = self.probe_present(&resolved.options);
                self.options = resolved.options.clone();
                self.opt_scroll = ListScroll::new();
                self.opt_scroll.len(self.options.len());
                self.view = View::Choosing;
                ModalAction::None
            }
            SearchEvent::Failed(msg) => {
                self.inflight = None;
                self.status = Some(msg.clone());
                // Fall back to a browsable view.
                self.view = if self.hits.is_empty() { View::Input } else { View::Results };
                ModalAction::None
            }
            SearchEvent::Downloaded(_) => ModalAction::None,
        }
    }
}

impl Default for SearchModal {
    fn default() -> Self {
        Self::new()
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Modal dimensions (columns × rows), clamped to the area by `draw_dialog`.
const MODAL_W: u16 = 70;
const MODAL_H: u16 = 20;

/// Draw the modal and return the dialog rects (for click hit-testing). The
/// query field is always drawn in the top content row; the list (hits or
/// download options) fills the rows below, and the final row carries the
/// "Results from IFDB" attribution.
pub fn draw_search_modal(
    modal: &mut SearchModal,
    area: Rect,
    cs: &ColorScheme,
    buf: &mut Buffer,
) -> DialogRects {
    let st = DialogStyle::from_colors(cs);
    let text = cs.theme.get("story_info_value").style;
    let dim = cs.theme.get("story_info_label").style;
    // Reversed, same idiom as every other caret text field in the app (the
    // save-name/text-entry dialogs, the command palette): a plain style with
    // no REVERSED modifier renders no visible caret block at all.
    let caret = st.frame.add_modifier(Modifier::REVERSED);

    let title = match modal.view {
        View::Choosing => "IFDB — choose a file",
        _ => "IFDB search",
    };

    let field = DialogField {
        label: "Search: ",
        value: modal.query.as_str(),
        cursor: modal.query.cursor,
        show_caret: modal.view == View::Input && modal.inflight.is_none(),
        dim: modal.query.is_empty(),
        text_style: text,
        dim_style: dim,
        caret_style: caret,
    };

    let spec = DialogSpec {
        title,
        placement: Placement::Centered { w: MODAL_W, h: MODAL_H },
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
        field: Some(field),
    };
    let rects = draw_dialog(buf, area, &spec, &st);

    // Content rows below the field: [status?] [list…] [attribution].
    let content = rects.content;
    if content.height <= 1 || content.width == 0 {
        return rects;
    }
    let inner = Rect::new(content.x, content.y + 1, content.width, content.height - 1);
    render_body(modal, inner, cs, buf);
    rects
}

fn render_body(modal: &mut SearchModal, area: Rect, cs: &ColorScheme, buf: &mut Buffer) {
    let sel_style = cs.theme.get("ifdb_result_selected").style;
    let row_style = cs.theme.get("ifdb_result").style;
    let meta_style = cs.theme.get("ifdb_result_meta").style;
    let marker_style = cs.theme.get("ifdb_download_marker").style;
    let present_style = cs.theme.get("ifdb_download_present").style;
    let marker_glyph = row_glyph(cs, "ifdb_download_marker", "⭳");
    let present_glyph = row_glyph(cs, "ifdb_download_present", "✓");
    let attrib_style = cs.theme.get("ifdb_attribution").style;
    let alert = cs.theme.get("alert").style;

    let mut y = area.y;
    let bottom = area.bottom();

    // Reserve the last row for the "Results from IFDB" attribution.
    let list_bottom = bottom.saturating_sub(1);

    // A status/error line, if present.
    if let Some(status) = &modal.status {
        if y < list_bottom {
            put_str(buf, area.x, y, area.width, status, alert);
            y += 1;
        }
    }

    // Hint line describing the current view's actions. SQ-0473: the "Popular
    // on IFDB" label doubles as the browse-list's own header (reusing the
    // existing meta style — no new selector) and documents the Esc ladder.
    let hint = match modal.view {
        // SQ-0691: the Tab hint only when there is a list to hop to.
        View::Input if !modal.hits.is_empty() => {
            "Type a title or author, Enter to search · Tab focus list."
        }
        View::Input => "Type a title or author, Enter to search.",
        View::Results if modal.busy() => "Fetching download options…",
        View::Results if modal.showing_seed => {
            "Popular on IFDB · ↑/↓ move · Enter download · o open page · Tab search · Esc close"
        }
        View::Results if !modal.seed_hits.is_empty() => {
            "↑/↓ move · Enter download · o open page · Tab search · Esc back to popular"
        }
        View::Results => "↑/↓ move · Enter download · o open page · Tab search · Esc edit query",
        View::Choosing => "↑/↓ move · Enter download this file · Esc back",
    };
    let busy_note = if modal.busy() && modal.view == View::Input { "Searching…" } else { hint };
    if y < list_bottom {
        put_str(buf, area.x, y, area.width, busy_note, meta_style);
        y += 1;
    }

    match modal.view {
        // SQ-0473: typing over a list keeps it visible underneath (the modal
        // stays on View::Input, editing `query`, while `hits` still holds
        // whatever list — seed or a prior search — was showing).
        View::Input | View::Results => {
            let rows = list_bottom.saturating_sub(y) as usize;
            modal.list_rows = rows.max(1);
            let start = window_start(modal.hit_scroll.display_offset(), modal.hits.len(), rows);
            let sel = modal.hit_scroll.selected;
            for (i, hit) in modal.hits.iter().enumerate().skip(start).take(rows) {
                render_hit_row(buf, area, y, hit, i == sel, row_style, sel_style, meta_style);
                y += 1;
            }
        }
        View::Choosing => {
            // SQ-0597: a file's description is what tells two downloads apart,
            // and it is far too long for the tail of a 70-column row (median 43
            // chars, and IFDB runs to 122), so it gets a row of its own. The
            // stride is uniform across the list — mixed row heights would make
            // the offset `ListScroll` maintains in *items* stop matching rows —
            // and drops back to 1 when no file in the list has anything to say.
            let stride = if modal.options.iter().any(|o| o.subtitle().is_some()) { 2 } else { 1 };
            let rows = list_bottom.saturating_sub(y) as usize;
            let items = (rows / stride).max(1);
            modal.list_rows = items;
            let start = window_start(modal.opt_scroll.display_offset(), modal.options.len(), items);
            let sel = modal.opt_scroll.selected;
            for (i, opt) in modal.options.iter().enumerate().skip(start).take(items) {
                let present = modal.opt_present.get(i).copied().unwrap_or(false);
                let (glyph, glyph_style) = if present {
                    (present_glyph.as_str(), present_style)
                } else {
                    (marker_glyph.as_str(), marker_style)
                };
                render_option_row(
                    buf, area, y, stride as u16, opt, i == sel, present, glyph, glyph_style,
                    row_style, sel_style, meta_style,
                );
                y += stride as u16;
            }
        }
    }

    // Attribution footer (honours IFDB's CC-BY metadata license).
    put_str(buf, area.x, list_bottom, area.width, "Results from IFDB (ifdb.org)", attrib_style);
}

/// Right-hand gutter reserved so a row's content never touches the dialog's
/// border column.
const ROW_MARGIN: u16 = 1;

/// One result row: "Title — Author", clipped with an ellipsis to leave room for
/// a right-aligned "★rating (year)" tail. The tail's width is reserved FIRST
/// and kept clear of the border gutter, then the title/author span is clipped
/// to end at least one space before it — so long text is truncated, never
/// overdrawn by the tail or bled into the margin. A tail that wouldn't leave
/// room for any title text is dropped rather than overflowing the row.
///
/// Selected rows fill their full width with `base` first (so the highlight
/// reaches both edges, matching the picker's other selected-row lists) and
/// draw the tail in that same style — otherwise it's invisible against the
/// reversed background.
fn render_hit_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    hit: &SearchHit,
    selected: bool,
    row_style: Style,
    sel_style: Style,
    meta_style: Style,
) {
    let base = if selected { sel_style } else { row_style };
    if selected {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    let content_right = area.right().saturating_sub(ROW_MARGIN);
    let content_w = content_right.saturating_sub(area.x);

    let tail = hit_rating(hit);
    let tail_w = tail.as_ref().map(|t| crate::textwidth::str_cells(t) as u16).unwrap_or(0);
    // Need the tail's width plus a 1-space gap before it; otherwise the row's
    // too narrow for both — drop the tail (never overflow).
    let (title_w, tail) = if tail_w > 0 && tail_w + 1 < content_w {
        (content_w - tail_w - 1, tail)
    } else {
        (content_w, None)
    };

    let line = format_hit(hit);
    let clipped = clip_with_ellipsis(&line, title_w);
    put_str(buf, area.x, y, title_w, &clipped, base);

    if let Some(t) = tail {
        let tail_style = if selected { base } else { meta_style };
        let tail_x = content_right - tail_w;
        put_str(buf, tail_x, y, tail_w, &t, tail_style);
    }
}

/// A row's themeable marker glyph, falling back to `default` when a theme
/// blanks it out. Same contract as the saves manager's portability glyph.
fn row_glyph(cs: &ColorScheme, selector: &str, default: &str) -> String {
    cs.theme
        .get(selector)
        .glyph
        .as_ref()
        .and_then(|g| g.single.as_deref())
        .unwrap_or(default)
        .to_string()
}

/// One download-chooser row: the file on the first line, and — when `stride` is
/// 2 — its IFDB description indented on the second (SQ-0597).
///
/// A selected row fills both of its lines edge to edge, the same idiom as
/// [`render_hit_row`], so the highlight reads as one block rather than as two
/// adjacent rows; that also means the description has to be drawn in the
/// selection style there, since a dim meta style is unreadable against it.
#[allow(clippy::too_many_arguments)]
fn render_option_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    stride: u16,
    opt: &DownloadOption,
    selected: bool,
    present: bool,
    glyph: &str,
    glyph_style: Style,
    row_style: Style,
    sel_style: Style,
    meta_style: Style,
) {
    let base = if selected { sel_style } else { row_style };
    if selected {
        for row in y..y + stride {
            for x in area.x..area.right() {
                if let Some(cell) = buf.cell_mut((x, row)) {
                    cell.set_symbol(" ").set_style(base);
                }
            }
        }
    }

    let content_w = area.right().saturating_sub(ROW_MARGIN).saturating_sub(area.x);

    // The marker glyph in its own style, then the file. A file the download
    // directory already holds says so outright rather than relying on the glyph
    // alone — downloading it again is allowed, it just lands as a "foo (2).z5".
    put_str(buf, area.x, y, 2, &format!("{glyph} "), if selected { base } else { glyph_style });
    let fmt = opt.format.as_deref().unwrap_or("story file");
    let mut line = format!("{}  ({})", opt.filename, fmt);
    if present {
        line.push_str(" · already downloaded");
    }
    let file_w = content_w.saturating_sub(2);
    put_str(buf, area.x + 2, y, file_w, &clip_with_ellipsis(&line, file_w), base);

    if stride < 2 {
        return;
    }
    // Indented past the glyph so it reads as belonging to the row above.
    const SUB_INDENT: u16 = 4;
    if let Some(sub) = opt.subtitle() {
        let sub_w = content_w.saturating_sub(SUB_INDENT);
        let style = if selected { base } else { meta_style };
        put_str(buf, area.x + SUB_INDENT, y + 1, sub_w, &clip_with_ellipsis(&sub, sub_w), style);
    }
}

/// Clip `s` to at most `width` display CELLS, appending an ellipsis if it had to
/// be shortened (the ellipsis itself costs one of those cells).
///
/// Cells, not chars (SQ-0655): a fullwidth IFDB title is two cells per char, so a
/// char-counted clip drew twice its budget — over the right-aligned rating tail
/// and on into the modal's border.
fn clip_with_ellipsis(s: &str, width: u16) -> String {
    crate::textwidth::clip_to_cols_ellipsis(s, width as usize)
}

/// "Title — Author" for a hit row.
fn format_hit(hit: &SearchHit) -> String {
    match &hit.author {
        Some(a) => format!("{} — {}", hit.title, a),
        None => hit.title.clone(),
    }
}

/// A compact rating/year tail like "★2.5 (1990)", or just the year, or None.
fn hit_rating(hit: &SearchHit) -> Option<String> {
    match (hit.star_rating, hit.published.as_deref()) {
        (Some(r), Some(y)) => Some(format!("★{r} {y}")),
        (Some(r), None) => Some(format!("★{r}")),
        (None, Some(y)) => Some(format!("({y})")),
        (None, None) => None,
    }
}

/// Clamp a [`ListScroll`] offset to a window that a `len`-item list can
/// actually fill. `ListScroll` only guarantees the offset is within the list;
/// after the list shrinks (a new search replaces a long result set with a short
/// one) an un-clamped offset would render a window hanging off the end, with
/// blank rows below a handful of items.
fn window_start(offset: usize, len: usize, rows: usize) -> usize {
    offset.min(len.saturating_sub(rows))
}

/// Write `s` (truncated to `width` cells) at `(x, y)` in `style`.
///
/// Advances by each glyph's DISPLAY width and blanks the second cell of a
/// double-width one, so `width` really is a cell budget: pairing one char with one
/// cell let a fullwidth string run past `x + width` on screen even after clipping
/// (SQ-0655). Grapheme clusters are written whole, so a combining mark or a ZWJ
/// emoji stays with its base.
fn put_str(buf: &mut Buffer, x: u16, y: u16, width: u16, s: &str, style: Style) {
    use unicode_segmentation::UnicodeSegmentation;
    let end = x.saturating_add(width);
    let mut cx = x;
    for g in s.graphemes(true) {
        let w = crate::textwidth::str_cells(g).max(1) as u16;
        if cx.saturating_add(w) > end {
            break;
        }
        if let Some(cell) = buf.cell_mut((cx, y)) {
            // IFDB text is untrusted; ratatui's cell API doesn't filter controls.
            let sym = if g.chars().all(char::is_control) { " " } else { g };
            cell.set_symbol(sym).set_style(style);
        }
        for k in 1..w {
            if let Some(cell) = buf.cell_mut((cx + k, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        cx += w;
    }
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use crate::ifdb_search::{DownloadOption, ResolvedGame, SearchEvent, SearchHit};

    fn hit(tuid: &str, title: &str) -> SearchHit {
        SearchHit {
            tuid: tuid.into(),
            title: title.into(),
            author: Some("Anon".into()),
            link: Some(format!("https://ifdb.org/viewgame?id={tuid}")),
            published: Some("1990".into()),
            star_rating: Some(3.0),
            num_ratings: Some(2),
            has_cover: false,
        }
    }

    fn opt(name: &str) -> DownloadOption {
        DownloadOption {
            filename: name.into(),
            url: format!("https://x/{name}"),
            format: Some("zcode".into()),
            title: None,
            desc: None,
        }
    }

    /// The same, carrying IFDB's per-file description (SQ-0597).
    fn opt_desc(name: &str, desc: &str) -> DownloadOption {
        DownloadOption { desc: Some(desc.into()), ..opt(name) }
    }

    /// Scroll animation off: these tests assert on settled offsets, and an
    /// eased `display_offset()` would be mid-tween when they look.
    fn anim() -> AnimationConfig {
        AnimationConfig { enabled: false, easing: crate::anim::Easing::EaseOut, scroll_ms: 0, ..Default::default() }
    }

    /// A resolved game with no iFiction record — the shape most existing
    /// tests want (they're exercising the options/download plumbing, not
    /// SQ-0474's metadata threading).
    fn resolved(options: Vec<DownloadOption>) -> ResolvedGame {
        ResolvedGame { options, record: None }
    }

    #[test]
    fn typing_then_enter_dispatches_a_search() {
        let mut m = SearchModal::new();
        for c in "zork".chars() {
            assert_eq!(m.on_key(KeyCode::Char(c), &anim()), ModalAction::None);
        }
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::Search("zork".into()));
        assert!(m.busy(), "a dispatched search marks the modal busy");
    }

    #[test]
    fn empty_query_enter_does_nothing() {
        let mut m = SearchModal::new();
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::None);
        assert!(!m.busy());
    }

    #[test]
    fn esc_from_input_closes() {
        let mut m = SearchModal::new();
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::Close);
    }

    /// SQ-0691: Tab/Shift-Tab toggle focus between the query field and the results list —
    /// keeping the query text and the list selection intact in both directions. Typing over
    /// the list drops into the query editor; before this, the only exits were Enter (an
    /// unwanted search) or Esc (closes the modal / falls down its ladder).
    #[test]
    fn tab_toggles_between_the_query_field_and_the_results_list() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha"), hit("bbb", "Beta")]));
        m.on_key(KeyCode::Down, &anim());
        assert_eq!(m.selected_hit_title(), Some("Beta"));

        // Typing over the list starts a query edit; Tab hops back to the list…
        m.on_key(KeyCode::Char('f'), &anim());
        assert_eq!(m.view, View::Input, "typing focuses the query");
        m.on_key(KeyCode::Tab, &anim());
        assert_eq!(m.view, View::Results, "Tab returns to the list");
        assert_eq!(m.selected_hit_title(), Some("Beta"), "…selection intact");

        // …and Shift-Tab hops to the query field, text intact, no search dispatched.
        assert_eq!(m.on_key(KeyCode::BackTab, &anim()), ModalAction::None);
        assert_eq!(m.view, View::Input, "Shift-Tab focuses the query");
        assert_eq!(m.query.as_str(), "zf", "the half-typed query survives the round trip");
        assert!(!m.busy(), "no search ran");
    }

    /// With no results to focus, Tab in the query field goes nowhere — there is no list.
    #[test]
    fn tab_with_no_results_stays_in_the_query_field() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        assert_eq!(m.on_key(KeyCode::Tab, &anim()), ModalAction::None);
        assert_eq!(m.view, View::Input, "nowhere to go: focus stays put");
    }

    #[test]
    fn results_arrive_then_enter_resolves_selected() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim()); // Search, busy
        let hits = vec![hit("aaa", "Alpha"), hit("bbb", "Beta")];
        assert_eq!(m.on_event(&SearchEvent::Results(hits)), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.selected_hit_title(), Some("Alpha"));
        m.on_key(KeyCode::Down, &anim());
        assert_eq!(m.selected_hit_title(), Some("Beta"));
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::Resolve("bbb".into()));
        assert!(m.busy());
    }

    #[test]
    fn empty_results_returns_to_input_with_a_status() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('x'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![]));
        assert!(!m.busy());
        // Back to input; a new keystroke edits the query again.
        assert_eq!(m.on_key(KeyCode::Char('y'), &anim()), ModalAction::None);
    }

    /// A lone playable file still goes through the chooser rather than
    /// downloading on the spot. It used to auto-download, which skipped the one
    /// screen that shows the file's description and whether the library already
    /// has it — so the commonest case was the one you could never inspect, and
    /// re-downloading a game you already had happened silently.
    #[test]
    fn one_download_option_still_opens_the_chooser() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim()); // Resolve
        let action = m.on_event(&SearchEvent::Options(resolved(vec![opt("a.z5")])));
        assert_eq!(action, ModalAction::None, "no download fires on its own");
        assert!(!m.busy(), "nothing is in flight — the user has not chosen yet");
        assert_eq!(m.view, View::Choosing);

        // And Enter on that single row downloads it, as it would with several.
        assert_eq!(
            m.on_key(KeyCode::Enter, &anim()),
            ModalAction::Download("https://x/a.z5".into())
        );
        assert!(m.busy());
    }

    /// The lone file is marked when the library already holds it — the whole
    /// reason this case stopped auto-downloading.
    #[test]
    fn a_lone_option_already_downloaded_says_so() {
        let dir = std::env::temp_dir().join(format!("bm_ifdb_lone_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("a.z5"), b"x").expect("seed an existing download");

        let mut m = SearchModal::new();
        m.set_download_dir(&dir);
        let rows = draw_chooser(&mut m, vec![opt("a.z5")]);
        let row = rows.iter().find(|r| r.contains("a.z5")).expect("the lone file is listed");
        assert!(row.contains("already downloaded"), "and is marked as present: {row:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0474: the iFiction record resolved alongside the options is cached
    /// for whichever `Download` follows, and handed to the picker exactly
    /// once — `take_pending_record` doesn't return it twice.
    #[test]
    fn resolved_record_is_cached_until_taken_once() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim()); // Resolve
        assert!(m.take_pending_record().is_none(), "nothing resolved yet");

        let record = Box::new(IFiction { title: Some("Alpha".into()), ..Default::default() });
        let action = m.on_event(&SearchEvent::Options(ResolvedGame {
            options: vec![opt("a.z5"), opt("a.z8")],
            record: Some(record.clone()),
        }));
        assert_eq!(action, ModalAction::None);
        assert_eq!(m.take_pending_record(), Some(record));
        assert!(m.take_pending_record().is_none(), "taken exactly once");
    }

    #[test]
    fn several_options_open_the_chooser() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim());
        let action = m.on_event(&SearchEvent::Options(resolved(vec![opt("a.z5"), opt("a.z8")])));
        assert_eq!(action, ModalAction::None);
        assert!(!m.busy());
        // Choosing view: Enter downloads the selected option.
        m.on_key(KeyCode::Down, &anim());
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::Download("https://x/a.z8".into()));
    }

    #[test]
    fn no_playable_option_offers_the_browser_fallback() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim());
        assert_eq!(m.on_event(&SearchEvent::Options(resolved(vec![]))), ModalAction::None);
        // 'o' opens the IFDB page.
        assert_eq!(
            m.on_key(KeyCode::Char('o'), &anim()),
            ModalAction::OpenInBrowser("https://ifdb.org/viewgame?id=aaa".into())
        );
    }

    #[test]
    fn esc_in_results_returns_to_query_editing() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::None);
        // Now in input view: Esc closes.
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::Close);
    }

    #[test]
    fn a_failed_event_shows_a_status_and_is_not_busy() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Failed("IFDB unreachable".into()));
        assert!(!m.busy());
        // A new keystroke edits the query.
        assert_eq!(m.on_key(KeyCode::Char('x'), &anim()), ModalAction::None);
    }

    #[test]
    fn stale_results_after_cancel_are_ignored() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim()); // busy, inflight=Search
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::None); // cancel
        assert!(!m.busy());
        // A late result must not jump to the results view.
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert_eq!(m.selected_hit_title(), None, "cancelled search result is dropped");
    }

    #[test]
    fn busy_ignores_typing_but_esc_cancels() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        assert!(m.busy());
        // Typing while busy is ignored.
        assert_eq!(m.on_key(KeyCode::Char('q'), &anim()), ModalAction::None);
        assert!(m.busy());
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::None);
        assert!(!m.busy(), "Esc abandons the in-flight request");
    }

    #[test]
    fn window_start_never_hangs_the_viewport_off_the_end_of_the_list() {
        assert_eq!(window_start(0, 10, 5), 0);
        assert_eq!(window_start(3, 10, 5), 3);
        assert_eq!(window_start(5, 10, 5), 5, "the last full window is allowed");
        assert_eq!(window_start(9, 10, 5), 5, "clamped back to the last full window");
        assert_eq!(window_start(4, 3, 5), 0, "a list shorter than the window starts at 0");
    }

    /// SQ-0598: the reported bug. Walking down past the viewport and back up
    /// again must move the *cursor* through the window, not drag the window
    /// under a cursor frozen on the bottom row. The old stateless window
    /// recompute pinned `selected` to the last row for every step of the way
    /// back up, which is what felt wrong.
    #[test]
    fn walking_back_up_a_long_list_moves_the_cursor_before_it_scrolls() {
        let mut m = SearchModal::new();
        m.open();
        let hits: Vec<_> = (0..20).map(|i| hit(&format!("t{i}"), &format!("Game {i}"))).collect();
        m.on_event(&SearchEvent::Results(hits));
        m.list_rows = 5;

        for _ in 0..9 {
            m.on_key(KeyCode::Down, &anim());
        }
        assert_eq!(m.hit_scroll.selected, 9);
        assert_eq!(m.hit_scroll.target_offset(), 5, "selection sits on the window's last row");

        // One step back up: the window must NOT move yet — the cursor has four
        // rows of headroom inside it.
        m.on_key(KeyCode::Up, &anim());
        assert_eq!(m.hit_scroll.selected, 8);
        assert_eq!(
            m.hit_scroll.target_offset(),
            5,
            "the list scrolled instead of moving the cursor — the SQ-0598 symptom"
        );

        // Only once the cursor reaches the window's TOP row does it scroll.
        for _ in 0..3 {
            m.on_key(KeyCode::Up, &anim());
        }
        assert_eq!((m.hit_scroll.selected, m.hit_scroll.target_offset()), (5, 5));
        m.on_key(KeyCode::Up, &anim());
        assert_eq!(
            (m.hit_scroll.selected, m.hit_scroll.target_offset()),
            (4, 4),
            "at the top row, one more Up scrolls the window by one"
        );
    }

    /// Home/End/PageUp/PageDown come free with `ListScroll` — the same keys the
    /// story picker binds, which is the point of sharing it.
    #[test]
    fn the_hit_list_pages_and_jumps_like_the_story_picker() {
        let mut m = SearchModal::new();
        m.open();
        let hits: Vec<_> = (0..30).map(|i| hit(&format!("t{i}"), &format!("Game {i}"))).collect();
        m.on_event(&SearchEvent::Results(hits));
        m.list_rows = 5;

        m.on_key(KeyCode::End, &anim());
        assert_eq!(m.hit_scroll.selected, 29, "End jumps to the last hit");
        m.on_key(KeyCode::Home, &anim());
        assert_eq!(m.hit_scroll.selected, 0, "Home jumps back to the first");
        m.on_key(KeyCode::PageDown, &anim());
        assert_eq!(m.hit_scroll.selected, 4, "PageDown advances ~a viewport (1-row overlap)");
        m.on_key(KeyCode::PageUp, &anim());
        assert_eq!(m.hit_scroll.selected, 0);
    }

    /// A shorter result set must not leave the viewport scrolled past its end.
    #[test]
    fn a_new_shorter_search_resets_the_scroll_offset() {
        let mut m = SearchModal::new();
        m.open();
        let many: Vec<_> = (0..20).map(|i| hit(&format!("t{i}"), &format!("Game {i}"))).collect();
        m.on_event(&SearchEvent::Results(many));
        m.list_rows = 5;
        m.on_key(KeyCode::End, &anim());
        assert!(m.hit_scroll.target_offset() > 0, "scrolled away from the top");

        m.on_key(KeyCode::Char('z'), &anim()); // start a new query
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("only", "Only Hit")]));
        assert_eq!(m.hit_scroll.selected, 0);
        assert_eq!(m.hit_scroll.target_offset(), 0, "a fresh list starts at the top");
    }

    // ── SQ-0831: the wheel scrolls the list, the keys move the cursor ────────

    /// The mirror image of the SQ-0598 test above: a KEY moves the cursor and
    /// the window follows; a wheel NOTCH moves the window and the cursor stays
    /// where it is until the window would carry it off screen.
    #[test]
    fn the_wheel_scrolls_the_hit_list_instead_of_moving_the_cursor() {
        let mut m = SearchModal::new();
        m.open();
        let hits: Vec<_> = (0..20).map(|i| hit(&format!("t{i}"), &format!("Game {i}"))).collect();
        m.on_event(&SearchEvent::Results(hits));
        m.list_rows = 5;

        m.on_key(KeyCode::Down, &anim());
        m.on_key(KeyCode::Down, &anim());
        assert_eq!((m.hit_scroll.selected, m.hit_scroll.target_offset()), (2, 0));

        m.on_wheel(1, &anim());
        assert_eq!(m.hit_scroll.target_offset(), 1, "the list scrolled one row");
        assert_eq!(m.hit_scroll.selected, 2, "…and the cursor did not move with it");

        // Two more notches and the cursor is at the window's top row; it rides
        // that edge from here rather than scrolling off the top.
        m.on_wheel(1, &anim());
        m.on_wheel(1, &anim());
        assert_eq!((m.hit_scroll.selected, m.hit_scroll.target_offset()), (3, 3));

        // Both ends: the offset stops at `len - rows`, never past the last hit.
        for _ in 0..50 {
            m.on_wheel(1, &anim());
        }
        assert_eq!(m.hit_scroll.target_offset(), 15, "20 hits, 5 rows → last window at 15");
        assert_eq!(m.hit_scroll.selected, 15);
        for _ in 0..50 {
            m.on_wheel(-1, &anim());
        }
        assert_eq!((m.hit_scroll.target_offset(), m.hit_scroll.selected), (0, 4));
    }

    #[test]
    fn the_wheel_does_nothing_on_a_list_that_fits_or_while_busy() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("a", "Alpha"), hit("b", "Beta")]));
        m.list_rows = 5;
        m.on_key(KeyCode::Down, &anim());
        m.on_wheel(1, &anim());
        assert_eq!((m.hit_scroll.target_offset(), m.hit_scroll.selected), (0, 1),
            "two hits in a five-row window: nothing to scroll, and the cursor stays");

        // Busy: the modal ignores everything but Esc, wheel included.
        let mut m = SearchModal::new();
        m.open();
        let hits: Vec<_> = (0..20).map(|i| hit(&format!("t{i}"), &format!("Game {i}"))).collect();
        m.on_event(&SearchEvent::Results(hits));
        m.list_rows = 5;
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        assert!(m.busy());
        m.on_wheel(1, &anim());
        assert_eq!(m.hit_scroll.target_offset(), 0, "a request in flight swallows the wheel");
    }

    /// The file chooser is the modal's other list, and gets the same rule.
    #[test]
    fn the_wheel_scrolls_the_download_chooser_too() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim()); // resolve
        let opts: Vec<_> = (0..12).map(|i| opt(&format!("f{i}.z5"))).collect();
        m.on_event(&SearchEvent::Options(resolved(opts)));
        assert_eq!(m.view, View::Choosing);
        m.list_rows = 4;

        m.on_wheel(1, &anim());
        assert_eq!(m.opt_scroll.target_offset(), 1, "the file list scrolled");
        assert_eq!(m.opt_scroll.selected, 1, "cursor pinned to the top of the window");
        for _ in 0..50 {
            m.on_wheel(1, &anim());
        }
        assert_eq!(m.opt_scroll.target_offset(), 8, "12 files, 4 rows → last window at 8");
    }

    // ── SQ-0473: seed list on open ──────────────────────────────────────────

    #[test]
    fn open_marks_the_modal_busy_and_asks_for_the_seed() {
        let mut m = SearchModal::new();
        assert!(!m.busy());
        assert_eq!(m.open(), ModalAction::Seed);
        assert!(m.busy(), "open() marks the modal busy, same as a dispatched search");
        assert_eq!(m.view, View::Input, "no seed rows yet — still the empty query box");
    }

    #[test]
    fn seed_results_land_on_the_browsable_seed_list() {
        let mut m = SearchModal::new();
        m.open();
        let seed = vec![hit("pop1", "Popular One"), hit("pop2", "Popular Two")];
        assert_eq!(m.on_event(&SearchEvent::Results(seed.clone())), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Results);
        assert!(m.showing_seed, "results view starts on the seed list");
        assert_eq!(m.selected_hit_title(), Some("Popular One"));
        // Download/resolve/open-in-browser work the same as any other hit row.
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::Resolve("pop1".into()));
    }

    #[test]
    fn empty_seed_result_degrades_to_the_plain_empty_query_box() {
        let mut m = SearchModal::new();
        m.open();
        assert_eq!(m.on_event(&SearchEvent::Results(vec![])), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Input, "nothing to browse — falls back silently");
        assert!(m.status.is_none(), "an empty seed isn't a user-facing error");
    }

    #[test]
    fn seed_fetch_failure_degrades_to_the_empty_query_box_with_a_status() {
        let mut m = SearchModal::new();
        m.open();
        assert_eq!(m.on_event(&SearchEvent::Failed("IFDB unreachable".into())), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Input, "never blocks opening the modal");
        assert_eq!(m.status.as_deref(), Some("IFDB unreachable"));
    }

    #[test]
    fn esc_from_the_seed_list_closes_the_modal() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("pop1", "Popular One")]));
        assert!(m.showing_seed);
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::Close, "one level shorter than before");
    }

    #[test]
    fn typing_over_the_seed_list_keeps_it_visible_until_enter_searches() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("pop1", "Popular One")]));
        assert_eq!(m.view, View::Results);

        // The first typed character starts editing the query, but the seed
        // rows stay put underneath (still readable off `hits`/`hit_sel`).
        assert_eq!(m.on_key(KeyCode::Char('z'), &anim()), ModalAction::None);
        assert_eq!(m.view, View::Input);
        assert_eq!(m.hits.len(), 1, "seed rows are not cleared by typing");
        assert_eq!(m.query.as_str(), "z");

        // Enter still runs a real search, same as ever.
        assert_eq!(m.on_key(KeyCode::Enter, &anim()), ModalAction::Search("z".into()));
        assert!(m.busy());
    }

    #[test]
    fn a_typed_search_replaces_the_seed_list_and_esc_returns_to_it() {
        let mut m = SearchModal::new();
        m.open();
        let seed = vec![hit("pop1", "Popular One")];
        m.on_event(&SearchEvent::Results(seed.clone()));
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim()); // busy, inflight = Search

        let searched = vec![hit("zzz", "Zork I"), hit("yyy", "Zork II")];
        assert_eq!(m.on_event(&SearchEvent::Results(searched.clone())), ModalAction::None);
        assert!(!m.showing_seed, "typed results replace the seed list");
        assert_eq!(m.selected_hit_title(), Some("Zork I"));

        // Esc from typed results goes back to the seed list, not an empty box.
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::None);
        assert!(m.showing_seed);
        assert_eq!(m.view, View::Results);
        assert_eq!(m.selected_hit_title(), Some("Popular One"));

        // A second Esc, now back on the seed list, closes.
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::Close);
    }

    #[test]
    fn esc_from_typed_results_with_no_cached_seed_falls_back_to_empty_query() {
        // The seed never loaded (e.g. it failed) — Esc from a typed search's
        // results has nothing to return to, so it falls back to the old
        // pre-SQ-0473 behaviour instead of a phantom "seed" screen.
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'), &anim());
        m.on_key(KeyCode::Enter, &anim());
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert!(!m.showing_seed);
        assert!(m.seed_hits.is_empty());
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::None);
        assert_eq!(m.view, View::Input);
        assert_eq!(m.on_key(KeyCode::Esc, &anim()), ModalAction::Close);
    }

    // ── Render: caret, selected-row meta, long-title layout ─────────────────

    #[test]
    fn query_caret_renders_reversed_mid_string_and_at_end() {
        let mut m = SearchModal::new();
        for c in "zork".chars() {
            m.on_key(KeyCode::Char(c), &anim());
        }
        m.on_key(KeyCode::Left, &anim()); // cursor now mid-string, before the final 'k'

        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        let rects = draw_search_modal(&mut m, area, &cs, &mut buf);
        let fr = rects.field.expect("field rect recorded");

        let label_len = "Search: ".chars().count() as u16;
        let mid_x = fr.x + label_len + 3; // cursor sits at char index 3 ("zor|k")
        assert!(
            buf.cell((mid_x, fr.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "mid-string caret cell should be reversed"
        );

        m.on_key(KeyCode::Right, &anim()); // back to end-of-text
        let mut buf2 = Buffer::empty(area);
        draw_search_modal(&mut m, area, &cs, &mut buf2);
        let end_x = fr.x + label_len + 4;
        assert!(
            buf2.cell((end_x, fr.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "end-of-text caret cell should be reversed"
        );
    }

    #[test]
    fn a_fullwidth_title_row_stays_inside_its_cell_budget() {
        // SQ-0655: clipping and drawing counted CHARS. A fullwidth (CJK) IFDB title
        // is two cells per char, so the "clipped" title drew twice its budget —
        // over the right-aligned rating tail and into the dialog border.
        // Distinct glyphs, so a dropped one is visible in the painted row.
        const TITLE_HEAD: &str = "日本語です漢字";
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("aaa", &TITLE_HEAD.repeat(9))]));

        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        let rects = draw_search_modal(&mut m, area, &cs, &mut buf);
        let content = rects.content;

        let y = (0..area.height)
            .find(|&y| (0..area.width).any(|x| buf.cell((x, y)).is_some_and(|c| c.symbol() == "日")))
            .expect("the CJK title row rendered");

        // What the terminal actually paints: a double-width symbol swallows the
        // following cell (ratatui's flush skips it), so a row written one char per
        // cell loses every second glyph on the way past its budget.
        let mut painted = String::new();
        let mut skip = 0usize;
        for x in content.x..content.right() {
            let sym = buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
            if skip > 0 {
                skip -= 1;
                continue;
            }
            skip = crate::textwidth::str_cells(&sym).saturating_sub(1);
            painted.push_str(&sym);
        }
        assert!(
            painted.starts_with(TITLE_HEAD),
            "the title paints its own glyphs in order, none swallowed: {painted:?}"
        );
        assert!(painted.contains('★'), "the rating tail survives the long title: {painted:?}");
        assert!(painted.contains('…'), "the title is marked as clipped: {painted:?}");
    }

    #[test]
    fn clip_with_ellipsis_budgets_cells_not_chars() {
        assert_eq!(clip_with_ellipsis("abcdef", 4), "abc…");
        assert_eq!(clip_with_ellipsis("日本語", 4), "日…", "two cells per glyph, plus the ellipsis");
        assert_eq!(clip_with_ellipsis("日本語", 6), "日本語", "an exact fit is not clipped");
        assert!(crate::textwidth::str_cells(&clip_with_ellipsis("日本語です", 7)) <= 7);
    }

    #[test]
    fn selected_result_row_keeps_its_rating_tail_visible() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha"), hit("bbb", "Beta")]));
        assert_eq!(m.hit_scroll.selected, 0);

        let cs = ColorScheme::terminal_default();
        let sel_style = cs.theme.get("ifdb_result_selected").style;
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        let rects = draw_search_modal(&mut m, area, &cs, &mut buf);

        // Row 0 (selected: "Alpha") is on the first body row under the field +
        // hint line, i.e. content.y + 2 in buffer coordinates. Find it by
        // scanning for the reversed title cell, then check a rating glyph
        // still appears further right on the same row.
        let title_row_y = (0..area.height)
            .find(|&y| {
                (0..area.width).any(|x| {
                    buf.cell((x, y))
                        .is_some_and(|c| c.symbol() == "A" && c.style().add_modifier.contains(Modifier::REVERSED))
                })
            })
            .expect("selected 'Alpha' row found");
        let row_text: String = (0..area.width)
            .filter_map(|x| buf.cell((x, title_row_y)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(row_text.contains("Alpha"), "selected row still shows its title: {row_text:?}");
        assert!(row_text.contains('★'), "selected row keeps its rating tail: {row_text:?}");
        // The row is highlighted edge-to-edge (the fill loop, not just the
        // glyphs) — checked at the content area's left edge, not the buffer's
        // (which would land on the dialog's own border column).
        // `Cell::set_style` patches onto the dialog's opaque background fill
        // rather than replacing it outright, so check the selected style's own
        // fg/modifiers landed rather than exact equality with the whole cell.
        let fill_style = buf.cell((rects.content.x, title_row_y)).unwrap().style();
        assert_eq!(fill_style.fg, sel_style.fg, "row fill uses the selected style's colour");
        assert!(
            fill_style.add_modifier.contains(Modifier::REVERSED),
            "row fill uses the selected style's reversed video"
        );
    }

    /// SQ-0598 at the level the user actually sees it: after stepping down past
    /// the viewport, one step back up must move the *highlight* and leave the
    /// visible window where it is. The old stateless window recompute did the
    /// opposite — it scrolled the list and left the highlight pinned to the
    /// bottom row — and this test discriminates the two without depending on
    /// any internal offset field.
    #[test]
    fn stepping_back_up_moves_the_highlight_not_the_window() {
        /// The rendered rows plus the y of the highlighted one.
        fn draw(m: &mut SearchModal) -> (Vec<String>, u16) {
            let cs = ColorScheme::terminal_default();
            let area = Rect::new(0, 0, MODAL_W, MODAL_H);
            let mut buf = Buffer::empty(area);
            draw_search_modal(m, area, &cs, &mut buf);
            let sel_y = (0..area.height)
                .find(|&y| {
                    (0..area.width).any(|x| {
                        buf.cell((x, y)).is_some_and(|c| {
                            c.style().add_modifier.contains(Modifier::REVERSED)
                                && c.symbol() != " "
                        })
                    })
                })
                .expect("a highlighted row is on screen");
            (rows_of(&buf, area), sel_y)
        }

        let mut m = SearchModal::new();
        m.open();
        let hits: Vec<_> =
            (0..30).map(|i| hit(&format!("t{i}"), &format!("Game{i:02}"))).collect();
        m.on_event(&SearchEvent::Results(hits));
        draw(&mut m); // establishes the real viewport height

        // Walk down well past the bottom of the window.
        for _ in 0..20 {
            m.on_key(KeyCode::Down, &anim());
        }
        let (before, sel_before) = draw(&mut m);
        let visible_before: Vec<&String> =
            before.iter().filter(|r| r.contains("Game")).collect();

        m.on_key(KeyCode::Up, &anim());
        let (after, sel_after) = draw(&mut m);
        let visible_after: Vec<&String> = after.iter().filter(|r| r.contains("Game")).collect();

        assert_eq!(
            visible_after, visible_before,
            "one step up from the bottom scrolled the list; the cursor should have moved \
             inside the window instead (SQ-0598)"
        );
        assert_eq!(
            sel_after + 1,
            sel_before,
            "the highlight stayed put while the list moved under it (SQ-0598)"
        );
    }

    /// Whole-buffer text, one string per row — enough for the chooser layout
    /// assertions, which are about *which row* a piece of text lands on.
    fn rows_of(buf: &Buffer, area: Rect) -> Vec<String> {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect()
            })
            .collect()
    }

    /// Drive the modal to `View::Choosing` with `options` and render it.
    fn draw_chooser(m: &mut SearchModal, options: Vec<DownloadOption>) -> Vec<String> {
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter, &anim()); // Resolve
        m.on_event(&SearchEvent::Options(resolved(options)));
        assert_eq!(m.view, View::Choosing, ">1 option opens the chooser");

        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        draw_search_modal(m, area, &cs, &mut buf);
        rows_of(&buf, area)
    }

    /// SQ-0597: the reported problem. Two files whose *filenames are identical*
    /// — IFDB really lists Photopia's v1.30 and competition builds both as
    /// `photopia.z5` — must still be told apart on screen, which is only
    /// possible from the description.
    #[test]
    fn the_chooser_shows_each_files_description_on_its_own_row() {
        let mut m = SearchModal::new();
        let rows = draw_chooser(
            &mut m,
            vec![
                opt_desc("photopia.z5", "Photopia v1.30"),
                opt_desc("photopia.z5", "Competition version"),
            ],
        );
        let text = rows.join("\n");
        assert!(text.contains("Photopia v1.30"), "first file's description is shown: {text}");
        assert!(text.contains("Competition version"), "second file's description too: {text}");

        // Each description sits on the row directly under its own filename,
        // rather than being crammed into the filename row's tail.
        let file_rows: Vec<usize> =
            (0..rows.len()).filter(|&i| rows[i].contains("photopia.z5")).collect();
        assert_eq!(file_rows.len(), 2, "both files listed: {rows:#?}");
        assert!(
            rows[file_rows[0] + 1].contains("Photopia v1.30"),
            "description belongs to the row above it: {rows:#?}"
        );
        assert!(
            rows[file_rows[1] + 1].contains("Competition version"),
            "description belongs to the row above it: {rows:#?}"
        );
        assert_eq!(file_rows[1] - file_rows[0], 2, "one file per two rows");
    }

    /// With nothing to say about any file, the list stays one row per file
    /// rather than paying for a blank second row each.
    #[test]
    fn the_chooser_stays_single_row_when_no_file_has_a_description() {
        let mut m = SearchModal::new();
        let rows = draw_chooser(&mut m, vec![opt("a.z5"), opt("b.z8")]);
        let a = rows.iter().position(|r| r.contains("a.z5")).expect("a.z5 listed");
        let b = rows.iter().position(|r| r.contains("b.z8")).expect("b.z8 listed");
        assert_eq!(b - a, 1, "no descriptions → adjacent rows: {rows:#?}");
    }

    /// A file the download directory already holds is marked, so a user can see
    /// they'd be making a duplicate before pressing Enter.
    #[test]
    fn a_file_already_in_the_download_directory_is_marked() {
        let dir = std::env::temp_dir().join(format!("bm_ifdb_present_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("have.z5"), b"x").expect("seed an existing download");

        let mut m = SearchModal::new();
        m.set_download_dir(&dir);
        let rows = draw_chooser(&mut m, vec![opt("have.z5"), opt("missing.z5")]);
        let have = rows.iter().find(|r| r.contains("have.z5")).expect("have.z5 listed").clone();
        let missing =
            rows.iter().find(|r| r.contains("missing.z5")).expect("missing.z5 listed").clone();

        assert!(have.contains("already downloaded"), "the present file says so: {have:?}");
        assert!(have.contains('✓'), "and carries the present marker: {have:?}");
        assert!(
            !missing.contains("already downloaded"),
            "a file we don't have is not marked: {missing:?}"
        );
        assert!(missing.contains('⭳'), "it keeps the ordinary download glyph: {missing:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without a download directory (the modal's default) nothing is claimed to
    /// be present — the probe must not guess.
    #[test]
    fn nothing_is_marked_present_without_a_download_directory() {
        let mut m = SearchModal::new();
        let rows = draw_chooser(&mut m, vec![opt("a.z5"), opt("b.z8")]);
        assert!(
            !rows.join("\n").contains("already downloaded"),
            "no directory set → no claims: {rows:#?}"
        );
    }

    #[test]
    fn long_title_row_ellipsizes_leaves_a_gap_and_keeps_the_tail_off_the_border() {
        let long = hit("aaa", &"A very very very long story title indeed".repeat(3));
        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        // Row width available to render_hit_row: MODAL_W minus the dialog's own
        // border/inset. Drive render_hit_row directly — it's the unit under
        // test for the layout fix, independent of the dialog chrome's exact inset.
        let row_area = Rect::new(2, 5, 40, 1);
        let row_style = cs.theme.get("ifdb_result").style;
        let sel_style = cs.theme.get("ifdb_result_selected").style;
        let meta_style = cs.theme.get("ifdb_result_meta").style;
        render_hit_row(&mut buf, row_area, row_area.y, &long, false, row_style, sel_style, meta_style);

        // The reserved right-margin gutter column (border-adjacent) stays
        // blank; everything else is the row's actual content.
        let gutter_x = row_area.right() - 1;
        let content_text: String = (row_area.x..gutter_x)
            .filter_map(|x| buf.cell((x, row_area.y)).map(|c| c.symbol().to_string()))
            .collect();
        let gutter_symbol = buf.cell((gutter_x, row_area.y)).unwrap().symbol().to_string();
        assert_eq!(gutter_symbol, " ", "border-margin column stays blank: {content_text:?}");

        assert!(content_text.contains('…'), "long title is ellipsized: {content_text:?}");
        let tail = hit_rating(&long).expect("rated hit has a tail");
        assert!(
            content_text.ends_with(&tail),
            "tail sits flush at the row's content edge: {content_text:?}"
        );
        let ellipsis_i = content_text.find('…').unwrap();
        let tail_i = content_text.find(&tail).unwrap();
        assert!(
            tail_i > ellipsis_i + 1,
            "at least one space between the ellipsis and the tail: {content_text:?}"
        );
    }

    #[test]
    fn narrow_row_drops_the_tail_instead_of_overflowing() {
        let h = hit("aaa", "Zork");
        let cs = ColorScheme::terminal_default();
        for w in 0..4u16 {
            let area = Rect::new(0, 0, w.max(1), 1);
            let mut buf = Buffer::empty(area);
            let row_area = Rect::new(0, 0, w, 1);
            let row_style = cs.theme.get("ifdb_result").style;
            let sel_style = cs.theme.get("ifdb_result_selected").style;
            let meta_style = cs.theme.get("ifdb_result_meta").style;
            // Must not panic for any degenerate width, selected or not.
            render_hit_row(&mut buf, row_area, 0, &h, false, row_style, sel_style, meta_style);
            render_hit_row(&mut buf, row_area, 0, &h, true, row_style, sel_style, meta_style);
        }
    }
}
