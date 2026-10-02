//! The "Download documents from IFDB" chooser (SQ-1680): a multi-select list of a
//! game's manuals, feelies and maps, over the links of its IFDB record, whose zip
//! rows open to show what is inside and whose text rows preview. What it does with
//! the network is [`crate::ifdb_documents`]'s business; this module is the UI half
//! — a state machine fed keys and [`DocEvent`]s that hands back the [`DocJob`]s to
//! run — plus its renderer, and [`DocumentsSession`], the chooser and its worker
//! together, which the story browser and the running game both host the same way.
//!
//! Conventions, as for the story download chooser it is modelled on: ↑/↓ (and
//! `j`/`k`, PageUp/PageDown, Home/End) move; Enter activates; Esc closes; Tab and
//! Shift-Tab walk the focus list → [ Download ] → [ Close ] and back. Space marks a
//! row, → (or `e`) opens a zip, ← closes it, `p` previews a text row. Enter on the
//! list downloads the marked rows, or the highlighted one when none is marked.
//!
//! One request is ever in flight: every job goes through [`DocumentsChooser::next_job`],
//! which hands out nothing while one is outstanding. A size probe is queued for the
//! row the cursor rests on, and a zip's directory only when the user opens it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use crate::colors::ColorScheme;
use crate::config::AnimationConfig;
use crate::data_roots::DataRoots;
use crate::ifdb_documents::{
    format_size, is_previewable, matches_title, DocEvent, DocJob, DocumentOption, DocumentWorker, DownloadItem,
    LinkKind, RowKey, ZipEntry, ZipListing,
};
use crate::ifdb_search::{IfdbGate, SearchSource};
use crate::ifdb_search_modal::{clip_with_ellipsis, put_str, row_glyph, window_start, ROW_MARGIN};
use crate::list_scroll::ListScroll;
use crate::render::dialog::{draw_dialog, ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement};

/// Shown when a game cannot have documents downloaded because IFDB does not know it.
pub const LINK_FIRST: &str = "Link to IFDB first";

#[derive(Debug, Clone, PartialEq, Eq)]
enum ZipState {
    Idle,
    Loading,
    Unknown,
    Listed(Vec<ZipEntry>),
    Failed(String),
}

struct LinkState {
    opt: DocumentOption,
    size: Option<u64>,
    /// A size probe has been queued (or has run) for this row.
    probed: bool,
    ranges: Option<bool>,
    zip: ZipState,
    expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Preview {
    Loading,
    Ready(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the record's links.
    Loading,
    Ready,
    Downloading { done: usize, total: usize },
}

/// Where Tab has put the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Download,
    Close,
}

/// What a key did to the dialog as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    Stay,
    Close,
}

pub struct DocumentsChooser {
    tuid: String,
    title: String,
    links: Vec<LinkState>,
    phase: Phase,
    scroll: ListScroll,
    /// Rows the list viewport last fitted, recorded by the renderer (which only
    /// has `&self`, hence the cell).
    list_rows: Cell<usize>,
    /// Where the dialog was last drawn, for a click that arrives with no rects
    /// of its own (the running game's mouse path).
    last_rects: RefCell<Option<DialogRects>>,
    checked: BTreeSet<RowKey>,
    previews: HashMap<RowKey, Preview>,
    focus: Focus,
    /// A line under the list: what happened, and whether it was a failure.
    status: Option<(String, bool)>,
    queue: VecDeque<DocJob>,
    inflight: bool,
    saved: bool,
}

impl DocumentsChooser {
    /// A chooser for the game `tuid` / `title`, with its first job (the record's
    /// links) already queued.
    pub fn new(tuid: &str, title: &str) -> Self {
        Self {
            tuid: tuid.to_string(),
            title: title.to_string(),
            links: Vec::new(),
            phase: Phase::Loading,
            scroll: ListScroll::new(),
            list_rows: Cell::new(1),
            last_rects: RefCell::new(None),
            checked: BTreeSet::new(),
            previews: HashMap::new(),
            focus: Focus::List,
            status: None,
            queue: VecDeque::from([DocJob::Resolve { tuid: tuid.to_string() }]),
            inflight: false,
            saved: false,
        }
    }

    /// The next job to run, if none is outstanding.
    pub fn next_job(&mut self) -> Option<DocJob> {
        if self.inflight {
            return None;
        }
        let job = self.queue.pop_front()?;
        self.inflight = true;
        Some(job)
    }

    /// True while a request is outstanding or waiting (keeps a host's loop ticking).
    pub fn busy(&self) -> bool {
        self.inflight || !self.queue.is_empty()
    }

    /// Whether files were saved since the last call — the host refreshes the info
    /// panel when so.
    pub fn take_saved(&mut self) -> bool {
        std::mem::take(&mut self.saved)
    }

    pub fn has_active_animation(&self) -> bool {
        self.scroll.has_active_animation()
    }

    pub fn finalize_if_done(&mut self) -> bool {
        self.scroll.finalize_if_done()
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    // ── Rows ─────────────────────────────────────────────────────────────────

    /// The visible rows: each link, and under an open zip its entries.
    fn rows(&self) -> Vec<RowKey> {
        let mut rows = Vec::new();
        for (i, l) in self.links.iter().enumerate() {
            rows.push((i, None));
            if let (true, ZipState::Listed(entries)) = (l.expanded, &l.zip) {
                rows.extend((0..entries.len()).map(|j| (i, Some(j))));
            }
        }
        rows
    }

    fn selected_key(&self) -> Option<RowKey> {
        self.rows().get(self.scroll.selected).copied()
    }

    fn entry(&self, key: RowKey) -> Option<&ZipEntry> {
        match (key, self.links.get(key.0).map(|l| &l.zip)) {
            ((_, Some(j)), Some(ZipState::Listed(entries))) => entries.get(j),
            _ => None,
        }
    }

    fn link_previewable(l: &LinkState) -> bool {
        l.opt.kind == LinkKind::Text
            && !matches!(
                std::path::Path::new(&l.opt.filename).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
                Some("doc" | "rtf")
            )
    }

    // ── Keys ─────────────────────────────────────────────────────────────────

    pub fn on_key(&mut self, code: KeyCode, anim: &AnimationConfig) -> KeyOutcome {
        if matches!(self.phase, Phase::Downloading { .. }) {
            return if code == KeyCode::Esc { KeyOutcome::Close } else { KeyOutcome::Stay };
        }
        self.status = None;
        match code {
            KeyCode::Esc => return KeyOutcome::Close,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::List => Focus::Download,
                    Focus::Download => Focus::Close,
                    Focus::Close => Focus::List,
                };
                return KeyOutcome::Stay;
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::List => Focus::Close,
                    Focus::Download => Focus::List,
                    Focus::Close => Focus::Download,
                };
                return KeyOutcome::Stay;
            }
            KeyCode::Enter => match self.focus {
                Focus::Close => return KeyOutcome::Close,
                Focus::Download | Focus::List => {
                    self.start_download();
                    return KeyOutcome::Stay;
                }
            },
            _ => {}
        }
        // Any other key belongs to the list; a button that had the focus gives it back.
        self.focus = Focus::List;
        let total = self.rows().len();
        if crate::list_scroll::nav_key(&mut self.scroll, code, total, self.list_rows.get(), anim) {
            self.want_probe();
            return KeyOutcome::Stay;
        }
        match code {
            KeyCode::Char(' ') => self.toggle_mark(),
            KeyCode::Right | KeyCode::Char('e') => self.expand(),
            KeyCode::Left => self.collapse(anim),
            KeyCode::Char('p') => self.request_preview(),
            _ => {}
        }
        self.want_probe();
        KeyOutcome::Stay
    }

    /// The wheel scrolls the list under the cursor, as in every chooser.
    pub fn on_wheel(&mut self, delta: isize, anim: &AnimationConfig) {
        let total = self.rows().len();
        self.scroll.len(total);
        self.scroll.scroll_by(delta, self.list_rows.get(), anim);
    }

    /// A click: ✕ or outside the dialog closes it, a button acts, anything else
    /// is swallowed (the list is keyboard-driven).
    pub fn on_click(&mut self, pos: Position, rects: &DialogRects) -> KeyOutcome {
        if matches!(self.phase, Phase::Downloading { .. }) {
            return if rects.close.is_some_and(|r| r.contains(pos)) || !rects.area.contains(pos) {
                KeyOutcome::Close
            } else {
                KeyOutcome::Stay
            };
        }
        if rects.close.is_some_and(|r| r.contains(pos)) || !rects.area.contains(pos) {
            return KeyOutcome::Close;
        }
        match rects.buttons.iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| *id) {
            Some(ButtonId::Ok) => self.start_download(),
            Some(ButtonId::Cancel) => return KeyOutcome::Close,
            _ => {}
        }
        KeyOutcome::Stay
    }

    /// [`on_click`](Self::on_click) against the rects of the last frame drawn.
    pub fn on_click_at(&mut self, pos: Position) -> KeyOutcome {
        let rects = self.last_rects.borrow().clone();
        match rects {
            Some(r) => self.on_click(pos, &r),
            None => KeyOutcome::Stay,
        }
    }

    fn toggle_mark(&mut self) {
        if let Some(key) = self.selected_key() {
            if !self.checked.remove(&key) {
                self.checked.insert(key);
            }
        }
    }

    fn expand(&mut self) {
        let Some((i, None)) = self.selected_key() else { return };
        let l = &mut self.links[i];
        if l.opt.kind != LinkKind::Archive {
            return;
        }
        l.expanded = true;
        if l.zip == ZipState::Idle {
            if l.ranges == Some(false) {
                l.zip = ZipState::Unknown;
            } else {
                l.zip = ZipState::Loading;
                self.queue.push_back(DocJob::ListZip { link: i, url: l.opt.url.clone() });
            }
        }
    }

    fn collapse(&mut self, anim: &AnimationConfig) {
        let Some((i, _)) = self.selected_key() else { return };
        self.links[i].expanded = false;
        let rows = self.rows();
        self.scroll.len(rows.len());
        if let Some(at) = rows.iter().position(|k| *k == (i, None)) {
            self.scroll.select(at, self.list_rows.get(), anim);
        }
    }

    fn request_preview(&mut self) {
        let Some(key) = self.selected_key() else { return };
        if self.previews.contains_key(&key) {
            return;
        }
        let l = &self.links[key.0];
        let job = match key.1 {
            None if Self::link_previewable(l) => {
                Some(DocJob::Preview { key, url: l.opt.url.clone(), entry: None })
            }
            Some(_) => self
                .entry(key)
                .filter(|e| is_previewable(&e.name))
                .map(|e| DocJob::Preview { key, url: l.opt.url.clone(), entry: Some(e.index) }),
            None => None,
        };
        match job {
            Some(job) => {
                self.previews.insert(key, Preview::Loading);
                self.queue.push_back(job);
            }
            None => self.status = Some(("Nothing to preview for this one".to_string(), false)),
        }
    }

    fn start_download(&mut self) {
        if self.phase != Phase::Ready {
            return;
        }
        let keys: Vec<RowKey> = if self.checked.is_empty() {
            self.selected_key().into_iter().collect()
        } else {
            self.checked.iter().copied().collect()
        };
        let items: Vec<DownloadItem> = keys
            .iter()
            .filter_map(|&key| {
                let l = self.links.get(key.0)?;
                Some(match key.1 {
                    None => DownloadItem::File { url: l.opt.url.clone(), filename: l.opt.filename.clone() },
                    Some(_) => DownloadItem::Entry { zip_url: l.opt.url.clone(), index: self.entry(key)?.index },
                })
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.phase = Phase::Downloading { done: 0, total: items.len() };
        self.queue.push_back(DocJob::Download { tuid: self.tuid.clone(), title: self.title.clone(), items });
    }

    /// Queue a size probe for the row the cursor rests on, once.
    fn want_probe(&mut self) {
        if let Some((i, None)) = self.selected_key() {
            let l = &mut self.links[i];
            if !l.probed {
                l.probed = true;
                self.queue.push_back(DocJob::Probe { link: i, url: l.opt.url.clone() });
            }
        }
    }

    // ── Worker events ────────────────────────────────────────────────────────

    pub fn on_event(&mut self, ev: &DocEvent) {
        if !matches!(ev, DocEvent::Progress { .. }) {
            self.inflight = false;
        }
        match ev {
            DocEvent::Resolved(Ok(docs)) => {
                self.phase = Phase::Ready;
                self.links = docs
                    .iter()
                    .cloned()
                    .map(|opt| LinkState { opt, size: None, probed: false, ranges: None, zip: ZipState::Idle, expanded: false })
                    .collect();
                self.scroll = ListScroll::new();
                self.scroll.len(self.links.len());
                if self.links.is_empty() {
                    self.status = Some(("IFDB lists no manuals, maps or other documents for this game".to_string(), false));
                }
            }
            DocEvent::Resolved(Err(e)) => {
                self.phase = Phase::Ready;
                self.status = Some((format!("Could not load the list: {e}"), true));
            }
            DocEvent::Probed { link, size, ranges } => {
                if let Some(l) = self.links.get_mut(*link) {
                    l.size = l.size.or(*size);
                    l.ranges = Some(*ranges);
                }
            }
            DocEvent::Listed { link, result } => {
                if let Some(l) = self.links.get_mut(*link) {
                    l.zip = match result {
                        Ok(ZipListing::Entries(e)) => ZipState::Listed(e.clone()),
                        Ok(ZipListing::Unknown) => {
                            l.ranges = Some(false);
                            ZipState::Unknown
                        }
                        Err(m) => ZipState::Failed(m.clone()),
                    };
                }
                let total = self.rows().len();
                self.scroll.len(total);
            }
            DocEvent::Previewed { key, result } => {
                self.previews.insert(
                    *key,
                    match result {
                        Ok(t) => Preview::Ready(t.clone()),
                        Err(e) => Preview::Failed(e.clone()),
                    },
                );
            }
            DocEvent::Progress { done, total, .. } => self.phase = Phase::Downloading { done: *done, total: *total },
            DocEvent::Finished { dir, saved, already, failed } => {
                self.phase = Phase::Ready;
                if !saved.is_empty() {
                    self.saved = true;
                    self.checked.clear();
                }
                let folder = dir.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
                let mut line = match (saved.len(), &folder) {
                    (0, _) => String::new(),
                    (1, Some(f)) => format!("Saved {} to {f}", saved[0]),
                    (n, Some(f)) => format!("Saved {n} files to {f}"),
                    (n, None) => format!("Saved {n} files"),
                };
                if !already.is_empty() {
                    if !line.is_empty() {
                        line.push_str(" · ");
                    }
                    line.push_str(&format!("Already in your documents: {}", already.join(", ")));
                }
                if let Some((what, why)) = failed.first() {
                    if !line.is_empty() {
                        line.push_str(" · ");
                    }
                    line.push_str(&format!("{} failed ({what}: {why})", failed.len()));
                }
                self.status = Some((line, !failed.is_empty()));
            }
        }
        self.want_probe();
    }

    // ── What a row shows ─────────────────────────────────────────────────────

    fn row_view(&self, key: RowKey) -> RowView {
        let checked = self.checked.contains(&key);
        match key.1 {
            None => {
                let l = &self.links[key.0];
                let marker = match (l.opt.kind, l.expanded) {
                    (LinkKind::Archive, true) => "▾ ",
                    (LinkKind::Archive, false) => "▸ ",
                    _ => "  ",
                };
                let tail = match l.size {
                    Some(s) => format!("{} · {}", l.opt.kind.label(), format_size(s)),
                    None => l.opt.kind.label().to_string(),
                };
                RowView {
                    checked,
                    indent: 0,
                    marker,
                    name: l.opt.filename.clone(),
                    tail,
                    spoiler: l.opt.spoiler,
                    title_match: matches_title(&l.opt.filename, &self.title),
                }
            }
            Some(_) => {
                let e = self.entry(key);
                let name = e.map(|e| e.name.clone()).unwrap_or_default();
                RowView {
                    checked,
                    indent: 4,
                    marker: "",
                    tail: e.map(|e| format_size(e.size)).unwrap_or_default(),
                    spoiler: e.is_some_and(|e| e.spoiler),
                    title_match: matches_title(&name, &self.title),
                    name,
                }
            }
        }
    }

    /// The pane under the list: a line about the highlighted row, then its preview
    /// (or the key that asks for one).
    fn pane_lines(&self) -> Vec<String> {
        let Some(key) = self.selected_key() else {
            return match self.phase {
                Phase::Loading => vec!["Asking IFDB what it lists for this game…".to_string()],
                _ => Vec::new(),
            };
        };
        let l = &self.links[key.0];
        let mut lines = Vec::new();
        match key.1 {
            Some(_) => lines.push(self.entry(key).map(|e| format!("in the zip: {}", e.path)).unwrap_or_default()),
            None => {
                let mut about = l.opt.subtitle().unwrap_or_default();
                if l.opt.spoiler {
                    about = format!("Spoiler — {about}");
                }
                lines.push(about);
                if l.opt.kind == LinkKind::Archive && l.expanded {
                    match &l.zip {
                        ZipState::Loading => lines.push("Reading the zip's contents…".to_string()),
                        ZipState::Unknown => lines.push(
                            "Contents unknown: this server cannot list a zip without sending all of it. Enter downloads the whole file."
                                .to_string(),
                        ),
                        ZipState::Failed(m) => lines.push(format!("Could not read the zip: {m}")),
                        ZipState::Listed(e) if e.is_empty() => lines.push("Nothing in this zip can be saved.".to_string()),
                        _ => {}
                    }
                }
            }
        }
        match self.previews.get(&key) {
            Some(Preview::Loading) => lines.push("Fetching a preview…".to_string()),
            Some(Preview::Failed(m)) => lines.push(format!("No preview: {m}")),
            Some(Preview::Ready(t)) => lines.extend(t.lines().map(str::to_string)),
            None => {}
        }
        lines
    }
}

struct RowView {
    checked: bool,
    indent: usize,
    marker: &'static str,
    name: String,
    tail: String,
    spoiler: bool,
    title_match: bool,
}

// ── Rendering ───────────────────────────────────────────────────────────────

const MODAL_W: u16 = 78;
const MODAL_H: u16 = 26;
/// Rows under the list: one about the row, the rest preview.
const PANE_H: u16 = 7;
const SPOILER_TAG: &str = " spoiler";

/// Draw the chooser and return its dialog rects (for click hit-testing).
pub fn draw_documents(ch: &DocumentsChooser, area: Rect, cs: &ColorScheme, buf: &mut Buffer) -> DialogRects {
    let st = DialogStyle::from_colors(cs);
    let title = format!("Documents — {}", ch.title);
    let focus = match ch.focus {
        Focus::List => None,
        Focus::Download => Some(0),
        Focus::Close => Some(1),
    };
    let download_label = match ch.phase {
        Phase::Downloading { .. } => "Downloading…",
        _ => "Download",
    };
    let buttons = [
        DialogButton { id: ButtonId::Ok, label: download_label },
        DialogButton { id: ButtonId::Cancel, label: "Close" },
    ];
    let spec = DialogSpec {
        title: &title,
        placement: Placement::Centered { w: MODAL_W, h: MODAL_H },
        buttons: &buttons,
        show_close: true,
        default: Some(ButtonId::Ok),
        focus,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    *ch.last_rects.borrow_mut() = Some(rects.clone());
    let c = rects.content;
    if c.height < 4 || c.width < 10 {
        return rects;
    }

    let row_style = cs.theme.get("ifdb_result").style;
    let sel_style = cs.theme.get("ifdb_result_selected").style;
    let meta = cs.theme.get("ifdb_result_meta").style;
    let checked_style = cs.theme.get("documents_checked").style;
    let spoiler_style = cs.theme.get("documents_spoiler").style;
    let match_style = cs.theme.get("documents_title_match").style;
    let preview_style = cs.theme.get("documents_preview").style;
    let attrib = cs.theme.get("ifdb_attribution").style;
    let alert = cs.theme.get("alert").style;
    let check_on = row_glyph(cs, "documents_checked", "✓");

    let hint = if ch.focus == Focus::List {
        "↑/↓ move · Space mark · → open zip · p preview · Enter download · Esc close"
    } else {
        "Tab/Shift-Tab move focus · Enter activate · Esc close"
    };
    put_str(buf, c.x, c.y, c.width, hint, meta);

    // Rows: hint, list, pane, status.
    let pane_h = PANE_H.min(c.height.saturating_sub(3));
    let list_top = c.y + 1;
    let status_y = c.bottom() - 1;
    let pane_top = status_y.saturating_sub(pane_h);
    let rows_h = pane_top.saturating_sub(list_top) as usize;
    ch.list_rows.set(rows_h.max(1));

    let keys = ch.rows();
    let start = window_start(ch.scroll.display_offset(), keys.len(), rows_h);
    let sel = ch.scroll.selected;
    let right = c.right().saturating_sub(ROW_MARGIN);
    for (n, key) in keys.iter().enumerate().skip(start).take(rows_h) {
        let y = list_top + (n - start) as u16;
        let v = ch.row_view(*key);
        let selected = n == sel;
        let base = if selected { sel_style } else { row_style };
        if selected {
            for x in c.x..c.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(base);
                }
            }
        }
        // Right-hand tail: kind and size, then the spoiler tag.
        let tag_w = if v.spoiler { SPOILER_TAG.len() as u16 } else { 0 };
        let tail_w = crate::textwidth::str_cells(&v.tail) as u16;
        let mut x_end = right;
        if v.spoiler {
            x_end -= tag_w.min(x_end - c.x);
            put_str(buf, x_end, y, tag_w, SPOILER_TAG, if selected { base } else { spoiler_style });
        }
        let tail_x = x_end.saturating_sub(tail_w + 1);
        put_str(buf, tail_x, y, tail_w + 1, &format!(" {}", v.tail), if selected { base } else { meta });
        // Left: mark, indent, zip marker, name.
        let mark = if v.checked { format!("[{check_on}]") } else { "[ ]".to_string() };
        put_str(buf, c.x, y, 3, &mark, if selected { base } else if v.checked { checked_style } else { base });
        let name_x = c.x + 4 + v.indent as u16;
        let name_w = tail_x.saturating_sub(name_x + 1);
        let name_style = if selected { base } else if v.title_match { match_style } else { base };
        let shown = format!("{}{}{}", v.marker, v.name, if v.title_match { " ★" } else { "" });
        put_str(buf, name_x, y, name_w, &clip_with_ellipsis(&shown, name_w), name_style);
    }

    // The pane.
    for (n, line) in ch.pane_lines().iter().take(pane_h as usize).enumerate() {
        let style = if n == 0 { meta } else { preview_style };
        put_str(buf, c.x, pane_top + n as u16, c.width, &clip_with_ellipsis(line, c.width), style);
    }

    // Status / progress / attribution.
    let (line, style) = match (&ch.phase, &ch.status) {
        (Phase::Downloading { done, total }, _) => (format!("Downloading {done}/{total}…"), meta),
        (_, Some((s, true))) => (s.clone(), alert),
        (_, Some((s, false))) => (s.clone(), meta),
        (Phase::Loading, None) => ("Asking IFDB…".to_string(), meta),
        _ => ("Links from IFDB (ifdb.org)".to_string(), attrib),
    };
    put_str(buf, c.x, status_y, c.width, &clip_with_ellipsis(&line, c.width), style);
    rects
}

// ── Chooser + worker ────────────────────────────────────────────────────────

/// A session for the running game `story_path` (and the story on a multi-story
/// disk image named by `disk_entry`), or the line to show instead: this player
/// has no library to save into, the game cannot be read, or IFDB does not know it.
pub fn session_for_story(
    roots: Option<&DataRoots>,
    story_path: &std::path::Path,
    disk_entry: Option<&str>,
    gate: IfdbGate,
    source: Box<dyn SearchSource>,
) -> Result<DocumentsSession, String> {
    let roots = roots.ok_or("No library to save documents into")?;
    let entry = crate::picker::resolve_entry_from(story_path, disk_entry, roots)
        .ok_or("Could not read this game's IFDB record")?;
    match (crate::documents::locate(roots, entry.meta.ifdb_tuid.as_deref(), &entry.title), &entry.meta.ifdb_tuid) {
        (crate::documents::Location::Unlinked, _) | (_, None) => Err(LINK_FIRST.to_string()),
        (_, Some(tuid)) => Ok(DocumentsSession::open(gate, source, roots.clone(), tuid, &entry.title)),
    }
}

/// The network source a host builds a session on: lanthorn's IFDB client.
pub fn default_source() -> Box<dyn SearchSource> {
    Box::new(crate::ifdb_search::IfdbSearchClient::new())
}

/// A chooser and the worker behind it, which the browser and the running game
/// host the same way: feed it keys, call [`pump`](Self::pump) every tick, draw it.
impl std::fmt::Debug for DocumentsSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DocumentsSession")
    }
}

pub struct DocumentsSession {
    pub chooser: DocumentsChooser,
    worker: DocumentWorker,
}

impl DocumentsSession {
    pub fn open(gate: IfdbGate, source: Box<dyn SearchSource>, roots: DataRoots, tuid: &str, title: &str) -> Self {
        let mut s = Self { chooser: DocumentsChooser::new(tuid, title), worker: DocumentWorker::new(gate, source, roots) };
        s.flush();
        s
    }

    fn flush(&mut self) {
        if let Some(job) = self.chooser.next_job() {
            self.worker.request(job);
        }
    }

    /// Hand the chooser what the worker has finished and start its next job;
    /// `true` when anything changed (a redraw is due).
    pub fn pump(&mut self) -> bool {
        let events = self.worker.drain();
        for ev in &events {
            self.chooser.on_event(ev);
        }
        self.flush();
        !events.is_empty()
    }

    pub fn on_key(&mut self, code: KeyCode, anim: &AnimationConfig) -> KeyOutcome {
        let out = self.chooser.on_key(code, anim);
        self.flush();
        out
    }

    pub fn on_click_at(&mut self, pos: Position) -> KeyOutcome {
        let out = self.chooser.on_click_at(pos);
        self.flush();
        out
    }

    pub fn busy(&self) -> bool {
        self.chooser.busy() || self.worker.busy()
    }
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use crate::ifdb_documents::parse_document_options;
    use crate::ifdb_documents::tests::{big_zip, files_in, roots, Host, ZORK};
    use crate::ifdb_search::{RangeProbe, ResolvedGame, SearchError, SearchHit};
    use crossterm::event::KeyCode::*;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const SOLS: &str = "http://www.ifarchive.org/if-archive/solutions/Sols1.zip";
    const TXT: &str = "http://www.ifarchive.org/if-archive/infocom/shipped-documentation/zork1.txt";

    fn anim() -> AnimationConfig {
        AnimationConfig { enabled: false, easing: crate::anim::Easing::EaseOut, scroll_ms: 0, ..Default::default() }
    }

    /// The fake host, shared so a test can read what the worker thread asked it.
    struct Shared(Arc<Host>);

    impl SearchSource for Shared {
        fn search(&self, q: &str) -> Result<Vec<SearchHit>, SearchError> {
            self.0.search(q)
        }
        fn hot(&self) -> Result<Vec<SearchHit>, SearchError> {
            self.0.hot()
        }
        fn download_options(&self, t: &str) -> Result<ResolvedGame, SearchError> {
            self.0.download_options(t)
        }
        fn download(&self, u: &str, d: &Path) -> Result<PathBuf, SearchError> {
            self.0.download(u, d)
        }
        fn probe_range(&self, u: &str) -> Result<RangeProbe, SearchError> {
            self.0.probe_range(u)
        }
        fn fetch_range(&self, u: &str, s: u64, l: u64) -> Result<Vec<u8>, SearchError> {
            self.0.fetch_range(u, s, l)
        }
        fn fetch_capped(&self, u: &str, c: u64) -> Result<Vec<u8>, SearchError> {
            self.0.fetch_capped(u, c)
        }
    }

    fn host(ranges: bool) -> Arc<Host> {
        let zip = big_zip(3, 2000, &[("Sols/ZorkI.txt", b"walkthrough text\nline2\n")]);
        let mut h = Host::new(ranges).with(SOLS, zip).with(TXT, b"GUE history\nline\n".to_vec());
        h.docs = parse_document_options(ZORK);
        Arc::new(h)
    }

    fn open(h: &Arc<Host>, tag: &str) -> (DocumentsSession, PathBuf, DataRoots) {
        let (home, roots) = roots(tag);
        let s = DocumentsSession::open(IfdbGate::default(), Box::new(Shared(Arc::clone(h))), roots.clone(), "abc123", "Zork I");
        (s, home, roots)
    }

    /// Pump until nothing is outstanding.
    fn settle(s: &mut DocumentsSession) {
        for _ in 0..5000 {
            s.pump();
            if !s.busy() {
                s.pump();
                if !s.busy() {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the chooser never settled");
    }

    fn press(s: &mut DocumentsSession, codes: &[crossterm::event::KeyCode]) {
        for c in codes {
            s.on_key(*c, &anim());
            settle(s);
        }
    }

    fn names(ch: &DocumentsChooser) -> Vec<String> {
        ch.rows().into_iter().map(|k| ch.row_view(k).name).collect()
    }

    fn screen(ch: &DocumentsChooser, w: u16, h: u16) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let cs = ColorScheme::terminal_default();
        draw_documents(ch, area, &cs, &mut buf);
        (0..h)
            .map(|y| (0..w).map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()).to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn it_opens_on_the_games_documents_and_probes_only_the_row_under_the_cursor() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-open");
        settle(&mut s);
        let n = names(&s.chooser);
        assert_eq!(n.len(), 12);
        assert_eq!(n[0], "Zork_Trilogy.zip");
        assert!(s.chooser.links[0].probed && s.chooser.links.iter().skip(1).all(|l| !l.probed), "only row 0 was probed");
        assert!(h.requested().is_empty(), "no zip was read: nothing was asked to be expanded");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_zip_opens_on_request_and_a_marked_entry_downloads_alone() {
        let h = host(true);
        let (mut s, home, roots) = open(&h, "chooser-zip");
        settle(&mut s);
        press(&mut s, &[Down, Down, Down, Down]);
        assert_eq!(names(&s.chooser)[4], "Sols1.zip");
        assert!(h.requested().is_empty(), "moving onto a zip does not open it");
        assert!(s.chooser.links[4].size.is_some(), "but its size was probed");

        press(&mut s, &[Right]);
        assert_eq!(h.requested().len(), 1, "opening it read the tail, once");
        let n = names(&s.chooser);
        assert_eq!(&n[5..9], ["game0.txt", "game1.txt", "game2.txt", "ZorkI.txt"]);
        let shown = screen(&s.chooser, 100, 40);
        assert!(shown.contains("ZorkI.txt ★"), "the entry named for the game is highlighted:\n{shown}");

        press(&mut s, &[Down, Down, Down, Down, Char(' ')]);
        assert_eq!(s.chooser.selected_key(), Some((4, Some(3))));
        assert!(screen(&s.chooser, 100, 40).contains("[✓]"));
        press(&mut s, &[Enter]);
        let (status, failure) = s.chooser.status.clone().expect("a result line");
        assert!(!failure && status.contains("Saved ZorkI.txt to Zork I [abc123]"), "{status}");
        assert_eq!(files_in(&roots.documents().join("Zork I [abc123]")), ["ZorkI.txt"]);
        assert!(s.chooser.take_saved(), "the host is told to refresh its panel");
        assert!(!s.chooser.take_saved(), "once");
        assert!(s.chooser.checked.is_empty(), "the marks are spent");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn enter_with_nothing_marked_downloads_the_highlighted_row_whole() {
        let h = host(true);
        let (mut s, home, roots) = open(&h, "chooser-enter");
        settle(&mut s);
        press(&mut s, &[Down, Down, Down, Down, Down, Down, Down, Down, Down]);
        assert_eq!(names(&s.chooser)[9], "zork1.txt");
        press(&mut s, &[Enter]);
        assert_eq!(files_in(&roots.documents().join("Zork I [abc123]")), ["zork1.txt"]);
        press(&mut s, &[Enter]);
        let (status, failure) = s.chooser.status.clone().expect("a result line");
        assert!(!failure && status == "Already in your documents: zork1.txt", "{status}");
        assert_eq!(files_in(&roots.documents().join("Zork I [abc123]")), ["zork1.txt"], "no copy");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn without_ranges_a_zip_says_contents_unknown_and_makes_no_request() {
        let h = host(false);
        let (mut s, home, _) = open(&h, "chooser-noranges");
        settle(&mut s);
        press(&mut s, &[Down, Down, Down, Down, Right]);
        assert_eq!(s.chooser.links[4].zip, ZipState::Unknown);
        assert!(h.requested().is_empty() && *h.whole.lock().unwrap() == 0, "nothing was fetched to find out");
        assert!(screen(&s.chooser, 100, 40).contains("Contents unknown"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn p_previews_a_text_row_and_says_so_for_one_that_is_not() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-preview");
        settle(&mut s);
        press(&mut s, &[Down, Down, Down, Down, Down, Down, Down, Down, Down, Char('p')]);
        assert!(screen(&s.chooser, 100, 40).contains("GUE history"));
        press(&mut s, &[Up, Up, Up, Up, Up, Up, Up, Up, Up, Char('p')]);
        assert_eq!(s.chooser.status.as_ref().map(|s| s.0.as_str()), Some("Nothing to preview for this one"), "a zip row");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_text_entry_inside_a_zip_previews_too() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-preview-entry");
        settle(&mut s);
        press(&mut s, &[Down, Down, Down, Down, Right, Down, Down, Down, Down, Char('p')]);
        assert!(screen(&s.chooser, 100, 40).contains("walkthrough text"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn tab_and_shift_tab_walk_the_focus_both_ways_and_enter_on_close_closes() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-focus");
        settle(&mut s);
        let ch = &mut s.chooser;
        assert_eq!(ch.focus(), Focus::List);
        ch.on_key(Tab, &anim());
        assert_eq!(ch.focus(), Focus::Download);
        ch.on_key(Tab, &anim());
        assert_eq!(ch.focus(), Focus::Close);
        ch.on_key(Tab, &anim());
        assert_eq!(ch.focus(), Focus::List);
        ch.on_key(BackTab, &anim());
        assert_eq!(ch.focus(), Focus::Close, "Shift-Tab reverses the cycle");
        assert_eq!(ch.on_key(Enter, &anim()), KeyOutcome::Close);
        ch.on_key(BackTab, &anim());
        assert_eq!(ch.focus(), Focus::Download);
        ch.on_key(Down, &anim());
        assert_eq!(ch.focus(), Focus::List, "a navigation key takes the focus back");
        assert_eq!(ch.on_key(Esc, &anim()), KeyOutcome::Close);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn at_most_one_job_is_ever_outstanding() {
        let mut ch = DocumentsChooser::new("t", "Zork I");
        assert!(matches!(ch.next_job(), Some(DocJob::Resolve { .. })));
        assert!(ch.next_job().is_none(), "the first has not come back");
        ch.on_event(&DocEvent::Resolved(Ok(parse_document_options(ZORK))));
        let probe = ch.next_job();
        assert!(matches!(probe, Some(DocJob::Probe { link: 0, .. })), "{probe:?}");
        ch.on_key(Down, &anim());
        ch.on_key(Down, &anim());
        assert!(ch.next_job().is_none(), "moving queues probes but starts none while one runs");
        ch.on_event(&DocEvent::Probed { link: 0, size: Some(5), ranges: true });
        assert!(matches!(ch.next_job(), Some(DocJob::Probe { link: 1, .. })));
    }

    #[test]
    fn it_draws_marks_sizes_and_the_spoiler_tag() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-draw");
        settle(&mut s);
        press(&mut s, &[Down, Char(' ')]);
        let shown = screen(&s.chooser, 100, 40);
        assert!(shown.contains("Documents — Zork I"), "{shown}");
        assert!(shown.contains("Zork_Trilogy.zip"));
        assert!(shown.contains("spoiler"), "{shown}");
        assert!(shown.contains("[✓]") && shown.contains("[ ]"));
        assert!(shown.contains("▸ zork1.zip"), "a closed zip is marked: {shown}");
        assert!(shown.contains("zip · "), "a probed size is shown: {shown}");
        assert!(shown.contains("Download") && shown.contains("Close"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_small_terminal_does_not_panic() {
        let h = host(true);
        let (mut s, home, _) = open(&h, "chooser-small");
        settle(&mut s);
        for (w, hh) in [(20u16, 6u16), (10, 4), (1, 1), (80, 5), (200, 60)] {
            screen(&s.chooser, w, hh);
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_failed_listing_says_why_and_a_game_without_documents_says_so() {
        let mut ch = DocumentsChooser::new("t", "Zork I");
        ch.on_event(&DocEvent::Resolved(Ok(Vec::new())));
        assert!(ch.status.as_ref().unwrap().0.contains("lists no manuals"));
        let mut ch = DocumentsChooser::new("t", "Zork I");
        ch.on_event(&DocEvent::Resolved(Err("IFDB unreachable".into())));
        assert_eq!(ch.status, Some(("Could not load the list: IFDB unreachable".to_string(), true)));
        assert!(screen(&ch, 100, 40).contains("Could not load the list"));
    }

    // ── from the running game ────────────────────────────────────────────────

    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00;
        buf[0x05] = 0x40;
        buf[0x06] = 0x00;
        buf[0x07] = 0x40;
        buf[0x0A] = 0x00;
        buf[0x0B] = 0x80;
        buf[0x0C] = 0x01;
        buf[0x0D] = 0x00;
        buf[0x0E] = 0x03;
        buf[0x0F] = 0x00;
        buf[0x08] = 0x04;
        buf[0x09] = 0x00;
        buf[0x18] = 0x00;
        buf[0x19] = 0x60;
        buf[0x12..0x18].copy_from_slice(b"000000");
        buf[0x0081] = 4;
        buf
    }

    #[test]
    fn the_running_game_opens_a_session_only_when_it_is_linked_to_ifdb() {
        let (home, roots) = roots("chooser-ingame");
        let story = home.join("game.z5");
        let bytes = minimal_v3_story();
        std::fs::write(&story, &bytes).unwrap();
        let source = || -> Box<dyn SearchSource> { Box::new(Shared(host(true))) };

        let err = |r: Result<DocumentsSession, String>| r.expect_err("an error line");
        assert_eq!(err(session_for_story(None, &story, None, IfdbGate::default(), source())), "No library to save documents into");
        assert_eq!(err(session_for_story(Some(&roots), &home.join("nope.z5"), None, IfdbGate::default(), source())), "Could not read this game's IFDB record");
        assert_eq!(err(session_for_story(Some(&roots), &story, None, IfdbGate::default(), source())), LINK_FIRST, "no sidecar: unlinked");

        // Link it, the way a fetch does.
        let game_dir = roots.catalogue_dir(&crate::storage::story_key_at(&story));
        let info = crate::story_info::StoryInfo {
            format_version: crate::story_info::FORMAT_VERSION,
            ifid: crate::ifid::compute_ifid(&bytes),
            fetched: Some(crate::story_info::FetchedMeta {
                scanned_at: "2026-07-16T00:00:00Z".into(),
                fetch_version: crate::story_info::FETCH_VERSION,
                source: "ifdb".into(),
                title: Some("Linked Game".into()),
                author: None,
                language: None,
                first_published: None,
                genre: None,
                description: None,
                ifdb_tuid: Some("tuid123".into()),
                ifdb_link: None,
                ifdb_rating: None,
                ifdb_rating_count: None,
                cover: None,
                not_found: false,
            }),
            probe: None,
        };
        crate::story_info::save(&game_dir, &info).unwrap();
        let mut s = session_for_story(Some(&roots), &story, None, IfdbGate::default(), source()).expect("linked: a session");
        settle(&mut s);
        assert_eq!(s.chooser.tuid, "tuid123");
        assert_eq!(s.chooser.links.len(), 12);
        let _ = std::fs::remove_dir_all(home);
    }
}
