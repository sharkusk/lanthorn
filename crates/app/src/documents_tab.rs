//! The Journal's Documents tab (SQ-1681): the running game's documents folder
//! (SQ-1679's manuals, maps and feelies), read without leaving the game.
//!
//! * a **list** of the folder's files (name, kind, size, a spoiler marker) under
//!   a header with the folder's `file://` link and the download / create buttons;
//! * a **pager** for text: decoded, line endings normalised, wrapped to the pane
//!   and re-wrapped when it resizes;
//! * an **image** view through the same kitty / sixel / half-block backend the
//!   story picker's covers use;
//! * everything else (PDFs, archives) hands off to the system viewer, with the
//!   `file://` link shown too.
//!
//! The state here is plain data plus a few `Cell`s the draw writes back (the
//! viewport, the last-drawn hit rects): the Journal is drawn from `&AppState`,
//! and the run loop must know where the rows and buttons landed to route a
//! click. Nothing polls the filesystem per frame — the list is read when the tab
//! is shown and when a download or import finishes ([`DocumentsTab::mark_dirty`]).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui_image::protocol::Protocol;

use crate::data_roots::DataRoots;
use crate::documents::{DocEntry, DocKind, Location};
use crate::input::Action;
use crate::state::AppState;

/// How much of a text document is read: past this the pager says so.
pub const TEXT_CAP: u64 = 4 * 1024 * 1024;
/// How much of an image file is read before giving up on decoding it.
pub const IMAGE_CAP: u64 = 32 * 1024 * 1024;
/// Two clicks on one row within this are a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The header's two buttons run these registry commands.
pub const CMD_DOWNLOAD: &str = "download-documents";
pub const CMD_CREATE: &str = "create-documents-folder";
const DOWNLOAD_LABEL: &str = " [ Download documents… ] ";
const CREATE_LABEL: &str = " [ Create documents folder ] ";
const CLOSE_LABEL: &str = " [ Close ] ";
const OPEN_LABEL: &str = " [ Open in system viewer ] ";
/// Said for a game that has no documents folder to show.
pub const UNLINKED_LINE: &str = "Link to IFDB for a documents folder";

// ── Text decoding ────────────────────────────────────────────────────────────

/// IBM code page 437's upper half, 0x80..=0xFF in order.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å',
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»',
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧',
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩',
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// Decode a document's bytes: UTF-8 when the whole thing is valid UTF-8,
/// otherwise **code page 437**, byte for byte.
///
/// Why CP437 and not Latin-1 for the fallback: a document that is not UTF-8 in a
/// game's folder is almost always from the era the games are (feelies, maps and
/// hint files typed on a DOS machine), and CP437 is what those used — it is also
/// the only candidate that renders their box-drawing maps instead of accented
/// gibberish. Its cost is Windows-era Latin-1/CP1252 text, whose accents come out
/// as box characters; plain ASCII, which is nearly all of them, is identical in
/// every candidate. A mixed file (valid UTF-8 with one stray byte) falls back as a
/// whole rather than guessing per sequence.
///
/// `truncated` says the bytes are a prefix of a longer file, so a multi-byte
/// sequence cut by the cap is trimmed instead of tipping the file into CP437.
pub fn decode_text(bytes: &[u8], truncated: bool) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(e) if truncated && e.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        Err(_) => bytes
            .iter()
            .map(|&b| if b < 0x80 { b as char } else { CP437_HIGH[(b - 0x80) as usize] })
            .collect(),
    }
}

/// Split decoded text into display lines: CRLF and lone CR (old Mac files) both
/// end a line like LF; tabs expand to 8-column stops; every other control
/// character (an ESC would reach the terminal) is dropped.
pub fn text_lines(text: &str) -> Vec<String> {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = norm
        .split('\n')
        .map(|l| {
            let mut out = String::with_capacity(l.len());
            let mut col = 0usize;
            for c in l.chars() {
                if c == '\t' {
                    let n = 8 - col % 8;
                    out.extend(std::iter::repeat_n(' ', n));
                    col += n;
                } else if !c.is_control() {
                    col += crate::textwidth::char_cells(c);
                    out.push(c);
                }
            }
            out
        })
        .collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop(); // the newline that ended the file is not a blank line
    }
    lines
}

/// Wrap `lines` to `width` columns: break at spaces, keep leading indentation,
/// and split a word longer than a line.
pub fn wrap_lines(lines: &[String], width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        wrap_one(line, width, &mut out);
    }
    out
}

fn wrap_one(line: &str, width: usize, out: &mut Vec<String>) {
    use crate::textwidth::{char_cells, str_cells};
    if str_cells(line) <= width {
        out.push(line.to_string());
        return;
    }
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut chars = line.chars().peekable();
    while chars.peek().is_some() {
        let mut spaces = String::new();
        while let Some(&c) = chars.peek().filter(|c| **c == ' ') {
            spaces.push(c);
            chars.next();
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek().filter(|c| **c != ' ') {
            word.push(c);
            chars.next();
        }
        let (sw, ww) = (spaces.len(), str_cells(&word));
        if cur_w + sw + ww <= width {
            cur.push_str(&spaces);
            cur.push_str(&word);
            cur_w += sw + ww;
            continue;
        }
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        // A fresh row: the word, split if it alone is wider than the row.
        for c in word.chars() {
            let cw = char_cells(c);
            if cur_w + cw > width && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            cur.push(c);
            cur_w += cw;
        }
    }
    out.push(cur);
}

// ── Views ────────────────────────────────────────────────────────────────────

/// A text document in the pager.
pub struct Pager {
    pub name: String,
    raw: Vec<String>,
    pub truncated: bool,
    /// The first visible wrapped line. Written by the wheel and keys; clamped by
    /// [`Self::max_scroll`], which the draw refreshes.
    pub scroll: Cell<usize>,
    max_scroll: Cell<usize>,
    viewport: Cell<usize>,
    wrapped: RefCell<Option<(u16, Vec<String>)>>,
}

impl Pager {
    pub fn new(name: &str, bytes: &[u8], truncated: bool) -> Pager {
        Pager {
            name: name.to_string(),
            raw: text_lines(&decode_text(bytes, truncated)),
            truncated,
            scroll: Cell::new(0),
            max_scroll: Cell::new(0),
            viewport: Cell::new(0),
            wrapped: RefCell::new(None),
        }
    }

    /// The wrapped lines for `width` columns, re-wrapped only when the width
    /// changed since the last call.
    pub fn wrapped_for(&self, width: u16) -> std::cell::Ref<'_, Vec<String>> {
        let stale = !matches!(&*self.wrapped.borrow(), Some((w, _)) if *w == width);
        if stale {
            *self.wrapped.borrow_mut() = Some((width, wrap_lines(&self.raw, width as usize)));
        }
        std::cell::Ref::map(self.wrapped.borrow(), |o| &o.as_ref().expect("just filled").1)
    }

    fn scroll_by(&self, delta: isize) {
        let next = (self.scroll.get() as isize + delta).clamp(0, self.max_scroll.get() as isize);
        self.scroll.set(next as usize);
    }
}

/// The area (columns, rows) and cell size (px) a protocol was fitted for.
type FitKey = (u16, u16, u16, u16);

/// An image document: decoded once, its protocol rebuilt only when the area it
/// is fitted to (or the terminal's cell size) changes.
pub struct ImageView {
    pub name: String,
    img: Arc<image::DynamicImage>,
    proto: RefCell<Option<(FitKey, Protocol)>>,
    /// The kitty upload last placed, so closing the view can free it.
    placed: Cell<Option<u32>>,
    /// The cell rect the last draw placed the image in.
    pub last_rect: Cell<Rect>,
}

/// A document the tab hands to the system (or can only point at).
pub struct ExternalView {
    pub name: String,
    pub path: PathBuf,
    /// Why it is shown this way, when that is not obvious from its kind.
    pub note: Option<String>,
    /// No viewer can be launched here (the browser-served Docker image).
    pub web: bool,
}

/// What the tab body is showing.
#[derive(Default)]
pub enum DocView {
    #[default]
    List,
    Text(Pager),
    Image(ImageView),
    External(ExternalView),
}

/// Where the last draw put things, for routing the mouse.
#[derive(Default, Clone, Debug)]
pub struct DocHits {
    pub area: Rect,
    pub rows: Vec<(usize, Rect)>,
    pub download: Option<Rect>,
    pub create: Option<Rect>,
    pub close: Option<Rect>,
    pub open_viewer: Option<Rect>,
    /// The `file://` link row(s): the whole row is clickable.
    pub path_link: Option<(Rect, String)>,
}

/// What opening a row came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// Nothing to open (no such row).
    Nothing,
    /// The document is spoiler-flagged: ask first. Carries its entry id.
    NeedsConfirm(String),
    /// Showing in the Journal (pager, image or the external card).
    Opened,
    /// A hint program (SQ-1690): not shown here. The caller switches to the
    /// Hints tab with this file.
    HintProgram(PathBuf),
    /// The file could not be read.
    Failed(String),
}

/// The tab's state.
pub struct DocumentsTab {
    pub entries: Vec<DocEntry>,
    pub location: Location,
    /// No library to hold documents at all (the host has no data roots).
    pub no_library: bool,
    /// The list is stale: read it again the next time the tab is on screen.
    pub dirty: bool,
    pub selected: usize,
    pub view: DocView,
    /// First visible list row; the wheel moves it, a keyboard move follows the selection.
    list_top: Cell<usize>,
    follow: Cell<bool>,
    list_viewport: Cell<usize>,
    last_click: Option<(usize, Instant)>,
    hits: RefCell<DocHits>,
    /// A line for the list's footer: the last failure, or what a button did.
    pub message: Option<String>,
}

impl Default for DocumentsTab {
    fn default() -> Self {
        DocumentsTab {
            entries: Vec::new(),
            location: Location::Unlinked,
            no_library: false,
            dirty: true,
            selected: 0,
            view: DocView::List,
            list_top: Cell::new(0),
            follow: Cell::new(true),
            list_viewport: Cell::new(0),
            last_click: None,
            hits: RefCell::new(DocHits::default()),
            message: None,
        }
    }
}

impl std::fmt::Debug for DocumentsTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentsTab")
            .field("entries", &self.entries.len())
            .field("dirty", &self.dirty)
            .finish()
    }
}

/// Whether `entry` is flagged as a spoiler: by the folder's index, or by its file
/// name with the same word rule the IFDB downloader flags options with.
pub fn is_spoiler(entry: &DocEntry) -> bool {
    entry.spoiler
}

fn kind_label(k: DocKind) -> &'static str {
    match k {
        DocKind::Pdf => "PDF",
        DocKind::Image => "image",
        DocKind::Text => "text",
        DocKind::HintProgram => "hint program \u{2014} opens in Hints tab",
        DocKind::Other => "file",
    }
}

fn read_capped(path: &Path, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(cap + 1).read_to_end(&mut buf)?;
    let truncated = buf.len() as u64 > cap;
    buf.truncate(cap as usize);
    Ok((buf, truncated))
}

impl DocumentsTab {
    /// Read the list again the next time the tab is shown.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Whether a viewer (not the list) is up.
    pub fn viewer_open(&self) -> bool {
        !matches!(self.view, DocView::List)
    }

    /// Install the folder's state and list its files, keeping the selection on the
    /// same file when it is still there. Clears the dirty flag.
    pub fn set_location(&mut self, location: Location) {
        let keep = self.entries.get(self.selected).map(|e| e.id.clone());
        self.entries = match &location {
            Location::Exists(dir) => crate::documents::list(dir).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.location = location;
        self.selected = keep
            .and_then(|id| self.entries.iter().position(|e| e.id == id))
            .unwrap_or(0)
            .min(self.entries.len().saturating_sub(1));
        self.follow.set(true);
        self.dirty = false;
    }

    /// Resolve the running game's folder and list it. A host with no library, or a
    /// game IFDB does not know, lists as unlinked.
    pub fn refresh(&mut self, story_path: &Path, disk_entry: Option<&str>, roots: Option<&DataRoots>) {
        self.no_library = roots.is_none();
        let loc = roots
            .and_then(|r| {
                let entry = crate::picker::resolve_entry_from(story_path, disk_entry, r)?;
                Some(crate::documents::locate(r, entry.meta.ifdb_tuid.as_deref(), &entry.title))
            })
            .unwrap_or(Location::Unlinked);
        self.set_location(loc);
    }

    /// Move the list selection by `delta` rows.
    pub fn select(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, max) as usize;
        self.follow.set(true);
    }

    /// Scroll whatever is up by `delta` lines (the list's first row, or the pager).
    pub fn scroll(&mut self, delta: isize) {
        match &self.view {
            DocView::Text(p) => p.scroll_by(delta),
            DocView::List => {
                let max = self.entries.len().saturating_sub(self.list_viewport.get().max(1));
                let next = (self.list_top.get() as isize + delta).clamp(0, max as isize);
                self.list_top.set(next as usize);
                self.follow.set(false);
            }
            _ => {}
        }
    }

    /// Scroll by a page (`dir` = ±1).
    pub fn scroll_page(&mut self, dir: i32) {
        let page = match &self.view {
            DocView::Text(p) => p.viewport.get(),
            _ => self.list_viewport.get(),
        };
        self.scroll(dir as isize * page.saturating_sub(1).max(1) as isize);
    }

    /// Open entry `idx`. `confirmed` skips the spoiler question; `web` is the
    /// browser-served mode where no viewer can be launched; `opener` is what
    /// launches one (the seam the tests fake).
    pub fn open_entry(
        &mut self,
        idx: usize,
        confirmed: bool,
        web: bool,
        opener: &mut dyn FnMut(&str),
    ) -> OpenOutcome {
        let Some(entry) = self.entries.get(idx).cloned() else { return OpenOutcome::Nothing };
        if !confirmed && is_spoiler(&entry) && entry.kind != DocKind::HintProgram {
            return OpenOutcome::NeedsConfirm(entry.id);
        }
        self.selected = idx;
        let external = |note: Option<String>| {
            DocView::External(ExternalView { name: entry.id.clone(), path: entry.path.clone(), note, web })
        };
        match entry.kind {
            // Never paged and never spoiler-asked: the Hints tab is the viewer, and
            // a hint program shows nothing until the player asks it something.
            DocKind::HintProgram => OpenOutcome::HintProgram(entry.path.clone()),
            DocKind::Text => match read_capped(&entry.path, TEXT_CAP) {
                Ok((bytes, truncated)) => {
                    self.view = DocView::Text(Pager::new(&entry.id, &bytes, truncated));
                    OpenOutcome::Opened
                }
                Err(e) => OpenOutcome::Failed(format!("Could not read {}: {e}", entry.id)),
            },
            DocKind::Image => match read_capped(&entry.path, IMAGE_CAP) {
                Ok((bytes, false)) => match crate::cover::decode(&bytes) {
                    Some(img) => {
                        self.view = DocView::Image(ImageView {
                            name: entry.id.clone(),
                            img: Arc::new(img),
                            proto: RefCell::new(None),
                            placed: Cell::new(None),
                            last_rect: Cell::new(Rect::default()),
                        });
                        OpenOutcome::Opened
                    }
                    None => {
                        self.view = external(Some("lanthorn cannot show this image format.".into()));
                        self.launch(web, opener);
                        OpenOutcome::Opened
                    }
                },
                Ok((_, true)) => {
                    self.view = external(Some("This image is too large to show here.".into()));
                    self.launch(web, opener);
                    OpenOutcome::Opened
                }
                Err(e) => OpenOutcome::Failed(format!("Could not read {}: {e}", entry.id)),
            },
            DocKind::Pdf | DocKind::Other => {
                self.view = external(None);
                self.launch(web, opener);
                OpenOutcome::Opened
            }
        }
    }

    /// Hand the open external document to the opener (never in web mode).
    pub fn launch(&self, web: bool, opener: &mut dyn FnMut(&str)) {
        if let (DocView::External(v), false) = (&self.view, web) {
            opener(&v.path.to_string_lossy());
        }
    }

    /// Back to the list. Returns the kitty image id an image view had placed, for
    /// the caller to free in the terminal.
    pub fn close_viewer(&mut self) -> Option<u32> {
        let id = self.release_image();
        self.view = DocView::List;
        self.follow.set(true);
        id
    }

    /// The kitty upload id the image view placed, forgotten here so it is freed once.
    pub fn release_image(&mut self) -> Option<u32> {
        match &self.view {
            DocView::Image(v) => v.placed.take(),
            _ => None,
        }
    }

    /// The click on list row `idx`: select it, or open it when it is the second
    /// click on it within the double-click window.
    pub fn click_row(&mut self, idx: usize, now: Instant) -> bool {
        if idx >= self.entries.len() {
            return false;
        }
        self.selected = idx;
        let double = matches!(self.last_click, Some((i, t)) if i == idx && now.duration_since(t) <= DOUBLE_CLICK);
        self.last_click = if double { None } else { Some((idx, now)) };
        double
    }

    /// The rects the last draw produced.
    pub fn hits(&self) -> DocHits {
        self.hits.borrow().clone()
    }
}

// ── Actions on the state ─────────────────────────────────────────────────────

/// Open the selected (or given) row for real: the system opener, the web-mode
/// check, and the spoiler question raised as an overlay.
pub fn open_selected(state: &mut AppState, idx: Option<usize>, confirmed: bool) {
    let idx = idx.unwrap_or(state.documents_tab.selected);
    let web = crate::opener::is_web_mode();
    match state.documents_tab.open_entry(idx, confirmed, web, &mut |t| crate::opener::open(t)) {
        OpenOutcome::NeedsConfirm(id) => {
            state.overlays.confirm_spoiler_document = Some(id);
            state.overlays.dialog_focus = 1; // Cancel is the safe default
        }
        OpenOutcome::Failed(why) => state.documents_tab.message = Some(why),
        OpenOutcome::HintProgram(path) => {
            state.documents_tab.message = None;
            crate::hints_tab::show_program(state, path);
        }
        OpenOutcome::Opened | OpenOutcome::Nothing => state.documents_tab.message = None,
    }
}

/// The spoiler dialog was answered.
pub fn answer_spoiler(state: &mut AppState, open: bool) {
    let Some(id) = state.overlays.confirm_spoiler_document.take() else { return };
    state.overlays.dialog_focus = 0;
    if open {
        if let Some(idx) = state.documents_tab.entries.iter().position(|e| e.id == id) {
            open_selected(state, Some(idx), true);
        }
    }
}

/// Close the viewer and free its terminal image.
pub fn close_viewer(state: &mut AppState) {
    if let Some(id) = state.documents_tab.close_viewer() {
        state.graphics_render.borrow_mut().queue_external_deletes([id]);
    }
}

/// Free the image view's terminal upload without closing the view: the Journal
/// moved to another tab, so nothing re-places it until the tab is back.
pub fn release_image(state: &mut AppState) {
    if let Some(id) = state.documents_tab.release_image() {
        state.graphics_render.borrow_mut().queue_external_deletes([id]);
    }
}

/// `create-documents-folder` for the running game: create the folder beside its
/// IFDB id, say what happened, and mark the list stale.
pub fn create_folder(state: &mut AppState, story_path: &Path) {
    let line = match state.data_roots.as_ref() {
        None => "No library to keep documents in".to_string(),
        Some(roots) => match crate::picker::resolve_entry_from(story_path, state.source.disk_entry.as_deref(), roots) {
            None => "Could not read this game's IFDB record".to_string(),
            Some(entry) => match crate::documents::locate(roots, entry.meta.ifdb_tuid.as_deref(), &entry.title) {
                Location::Unlinked => UNLINKED_LINE.to_string(),
                Location::Exists(p) => format!("Documents folder already exists: {}", p.display()),
                Location::Missing(_) => {
                    let tuid = entry.meta.ifdb_tuid.as_deref().unwrap_or_default();
                    match crate::documents::ensure_documents_dir(roots, tuid, &entry.title) {
                        Ok(p) => format!("Created documents folder: {}", p.display()),
                        Err(e) => format!("Could not create the documents folder: {e}"),
                    }
                }
            },
        },
    };
    state.documents_tab.mark_dirty();
    state.set_status(line);
}

/// Re-read the list when the tab is on screen and stale. Called once per loop turn.
pub fn refresh_if_needed(state: &mut AppState, story_path: &Path) -> bool {
    if !state.documents_tab_visible() || !state.documents_tab.dirty {
        return false;
    }
    let disk = state.source.disk_entry.clone();
    let roots = state.data_roots.clone();
    state.documents_tab.refresh(story_path, disk.as_deref(), roots.as_ref());
    true
}

// ── Mouse ────────────────────────────────────────────────────────────────────

/// What a mouse event over the tab means.
pub enum DocMouse {
    Action(Action),
    /// Run a registry command through the ordinary slash pipeline.
    Command(&'static str),
}

/// Route a mouse event over the Documents tab, or `None` when it is not over it.
/// The tab claims every event inside its body, so a click here is never a map
/// click or a story selection.
pub fn mouse_action(state: &AppState, m: &MouseEvent) -> Option<DocMouse> {
    if !state.documents_tab_visible() || state.any_modal_overlay_open() {
        return None;
    }
    let hits = state.documents_tab.hits();
    let pt = ratatui::layout::Position { x: m.column, y: m.row };
    if !hits.area.contains(pt) {
        return None;
    }
    let hit = |r: &Option<Rect>| r.is_some_and(|r| r.contains(pt));
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if hit(&hits.download) {
                return Some(DocMouse::Command(CMD_DOWNLOAD));
            }
            if hit(&hits.create) {
                return Some(DocMouse::Command(CMD_CREATE));
            }
            if hit(&hits.close) {
                return Some(DocMouse::Action(Action::DocTabClose));
            }
            if hit(&hits.open_viewer) {
                return Some(DocMouse::Action(Action::DocTabOpen(None)));
            }
            if let Some((r, url)) = &hits.path_link {
                if r.contains(pt) {
                    return Some(DocMouse::Action(Action::DocTabOpenLink(url.clone())));
                }
            }
            match hits.rows.iter().find(|(_, r)| r.contains(pt)) {
                Some((idx, _)) => Some(DocMouse::Action(Action::DocTabClickRow(*idx))),
                None => Some(DocMouse::Action(Action::None)),
            }
        }
        _ => match crate::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
            Some(d) => Some(DocMouse::Action(Action::DocTabScroll(d as i32))),
            None => Some(DocMouse::Action(Action::None)),
        },
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

struct Styles {
    row: Style,
    selected: Style,
    meta: Style,
    desc: Style,
    spoiler: Style,
    header: Style,
    button: Style,
    hint: Style,
    pager: Style,
    pager_title: Style,
    pager_notice: Style,
}

impl Styles {
    fn of(state: &AppState) -> Styles {
        let t = |name: &str| state.colors.theme.get(name).style;
        Styles {
            row: t("journal.docs.row"),
            selected: t("journal.docs.row:selected"),
            meta: t("journal.docs.meta"),
            desc: t("journal.docs.desc"),
            spoiler: t("journal.docs.spoiler"),
            header: t("journal.docs.header"),
            button: t("journal.docs.button"),
            hint: t("journal.docs.hint"),
            pager: t("journal.docs.pager"),
            pager_title: t("journal.docs.pager_title"),
            pager_notice: t("journal.docs.pager_notice"),
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

fn put(buf: &mut Buffer, area: Rect, y: u16, x: u16, text: &str, style: Style) -> Rect {
    crate::render::draw_str_clipped(buf, x, y, text, style, area);
    let w = (crate::textwidth::str_cells(text) as u16).min(area.right().saturating_sub(x));
    Rect::new(x, y, w, 1)
}

/// An OSC 8 hyperlink over the whole of `rect`'s label — the same construction the
/// story info panel uses for its folder link (see `picker_ui`), including the
/// forced one-column width on the cell that carries the escape.
fn put_link(buf: &mut Buffer, rect: Rect, text: &str, url: &str, style: Style) {
    if rect.width == 0 {
        return;
    }
    let link = hyperrat::Link::new(text, url).style(style);
    ratatui::widgets::Widget::render(link, rect, buf);
    if let Some(first) = buf.cell_mut(ratatui::layout::Position::new(rect.x, rect.y)) {
        first.set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
            std::num::NonZeroU16::new(1).expect("1 is not zero"),
        ));
    }
}

/// Draw the tab into `area` (the Journal body) and record where things landed.
pub fn draw(state: &AppState, area: Rect, buf: &mut Buffer) {
    let st = Styles::of(state);
    let tab = &state.documents_tab;
    let mut hits = DocHits { area, ..DocHits::default() };
    if area.width == 0 || area.height == 0 {
        *tab.hits.borrow_mut() = hits;
        return;
    }
    fill(buf, area, st.row);
    match &tab.view {
        DocView::List => draw_list(state, tab, &st, area, buf, &mut hits),
        DocView::Text(p) => draw_pager(p, &st, area, buf, &mut hits),
        DocView::Image(v) => draw_image(state, v, &st, area, buf, &mut hits),
        DocView::External(v) => draw_external(v, &st, area, buf, &mut hits),
    }
    *tab.hits.borrow_mut() = hits;
}

fn draw_list(state: &AppState, tab: &DocumentsTab, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits) {
    let mut y = area.y;
    let x = area.x + 1;
    // Header: the folder, as a link when it exists.
    match &tab.location {
        Location::Unlinked => {
            let line = if tab.no_library { "No library to keep documents in" } else { UNLINKED_LINE };
            put(buf, area, y, x, line, st.hint);
            return finish_list(tab, st, area, buf, hits, y + 2, false);
        }
        Location::Exists(p) => {
            let label = format!("Folder: {}", p.display());
            let url = crate::documents::file_url(p);
            let rect = Rect::new(x, y, area.width.saturating_sub(1), 1);
            put_link(buf, rect, &label, &url, st.header);
            hits.path_link = Some((rect, url));
        }
        Location::Missing(p) => {
            let label = format!("Folder: {} (not created)", p.display());
            put(buf, area, y, x, &label, st.header);
        }
    }
    y += 1;
    // Buttons, one unstyled cell apart; the second wraps to its own row when
    // both do not fit across the pane.
    let mut bx = x;
    if y < area.bottom() {
        if matches!(tab.location, Location::Missing(_)) {
            let r = put(buf, area, y, bx, CREATE_LABEL, st.button);
            hits.create = Some(r);
            bx += r.width + 1;
            if bx + crate::textwidth::str_cells(DOWNLOAD_LABEL) as u16 > area.right() {
                bx = x;
                y += 1;
            }
        }
        if y < area.bottom() {
            hits.download = Some(put(buf, area, y, bx, DOWNLOAD_LABEL, st.button));
        }
    }
    finish_list(tab, st, area, buf, hits, y + 2, true);
    let _ = state;
}

fn finish_list(tab: &DocumentsTab, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits, top: u16, has_folder: bool) {
    if top >= area.bottom() {
        return;
    }
    let x = area.x + 1;
    if tab.entries.is_empty() {
        if has_folder {
            let msg = if matches!(tab.location, Location::Exists(_)) {
                "No documents yet. Drop files into the folder above, or download them from IFDB."
            } else {
                "Create the folder, then drop files in it or download them from IFDB."
            };
            let w = area.width.saturating_sub(2).max(1) as usize;
            for (i, l) in wrap_lines(&[msg.to_string()], w).iter().enumerate() {
                if top + (i as u16) < area.bottom() {
                    put(buf, area, top + i as u16, x, l, st.hint);
                }
            }
        }
        return;
    }
    // The footer takes the last row: a failure or the key hint.
    let footer_y = area.bottom() - 1;
    let list_bottom = if footer_y > top { footer_y } else { area.bottom() };
    // A row is two lines when any file has a description to show, so every row
    // is the same height and scrolling stays in whole rows.
    let row_h: u16 = if tab.entries.iter().any(|e| e.subtitle().is_some()) && list_bottom - top >= 2 { 2 } else { 1 };
    let viewport = (list_bottom.saturating_sub(top) / row_h) as usize;
    tab.list_viewport.set(viewport);
    let max_top = tab.entries.len().saturating_sub(viewport.max(1));
    let mut first = tab.list_top.get().min(max_top);
    if tab.follow.get() && viewport > 0 {
        if tab.selected < first {
            first = tab.selected;
        } else if tab.selected >= first + viewport {
            first = tab.selected + 1 - viewport;
        }
    }
    tab.list_top.set(first);
    for (i, e) in tab.entries.iter().enumerate().skip(first).take(viewport) {
        let ry = top + (i - first) as u16 * row_h;
        let row = Rect::new(area.x, ry, area.width, row_h);
        let on = i == tab.selected;
        let base = if on { st.selected } else { st.row };
        fill(buf, row, base);
        let spoiler = is_spoiler(e);
        let meta = format!("{:<5} {:>7}", kind_label(e.kind), crate::ifdb_documents::format_size(e.size));
        let flag = if spoiler { " spoiler" } else { "" };
        let right_w = crate::textwidth::str_cells(&meta) + flag.len() + 1;
        let name_w = (area.width as usize).saturating_sub(right_w + 2);
        let name = crate::textwidth::clip_to_cols_ellipsis(&e.id, name_w);
        let marker = if on { "▸" } else { " " };
        put(buf, area, ry, area.x, marker, base);
        put(buf, area, ry, x, &name, base);
        let meta_x = area.right().saturating_sub(right_w as u16);
        let meta_style = if on { st.selected } else { st.meta };
        let r = put(buf, area, ry, meta_x, &meta, meta_style);
        if spoiler {
            let sp_style = if on { st.selected.patch(st.spoiler) } else { st.spoiler };
            put(buf, area, ry, r.right(), flag, sp_style);
        }
        if row_h == 2 {
            if let Some(sub) = e.subtitle() {
                let line = crate::textwidth::clip_to_cols_ellipsis(&sub, (area.width as usize).saturating_sub(4));
                put(buf, area, ry + 1, x + 2, &line, if on { st.selected } else { st.desc });
            }
        }
        hits.rows.push((i, row));
    }
    if footer_y > top {
        let line = tab.message.clone().unwrap_or_else(|| "Shift+↑/↓ select · Shift+→ open · or double-click".to_string());
        let style = if tab.message.is_some() { st.pager_notice } else { st.hint };
        put(buf, area, footer_y, x, &crate::textwidth::clip_to_cols_ellipsis(&line, area.width.saturating_sub(2) as usize), style);
    }
}

/// The title row shared by the three viewers: the name and a close button.
fn draw_title(name: &str, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits) {
    let close_w = crate::textwidth::str_cells(CLOSE_LABEL) as u16;
    let close_x = area.right().saturating_sub(close_w).max(area.x);
    let name_w = close_x.saturating_sub(area.x + 1) as usize;
    put(buf, area, area.y, area.x + 1, &crate::textwidth::clip_to_cols_ellipsis(name, name_w), st.pager_title);
    hits.close = Some(put(buf, area, area.y, close_x, CLOSE_LABEL, st.button));
}

fn draw_pager(p: &Pager, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits) {
    draw_title(&p.name, st, area, buf, hits);
    let notice = p.truncated.then_some("— only the first 4 MB is shown —");
    let top = area.y + 1;
    let bottom = area.bottom().saturating_sub(u16::from(notice.is_some() && area.height > 2));
    let viewport = bottom.saturating_sub(top) as usize;
    let width = area.width.saturating_sub(2).max(1);
    let lines = p.wrapped_for(width);
    p.viewport.set(viewport);
    let max = lines.len().saturating_sub(viewport);
    p.max_scroll.set(max);
    let first = p.scroll.get().min(max);
    p.scroll.set(first);
    for (i, l) in lines.iter().skip(first).take(viewport).enumerate() {
        put(buf, area, top + i as u16, area.x + 1, l, st.pager);
    }
    if let Some(n) = notice.filter(|_| area.height > 2) {
        put(buf, area, area.bottom() - 1, area.x + 1, n, st.pager_notice);
    }
}

fn draw_image(state: &AppState, v: &ImageView, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits) {
    draw_title(&v.name, st, area, buf, hits);
    let body = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
    let Some(picker) = state.game_picker.as_ref().filter(|_| body.width > 0 && body.height > 0) else {
        return;
    };
    let fs = picker.font_size();
    let key = (body.width, body.height, fs.width, fs.height);
    let stale = !matches!(&*v.proto.borrow(), Some((k, _)) if *k == key);
    if stale {
        let built = crate::render::graphics::fitted_protocol(picker, &v.img, Size::new(body.width, body.height), false);
        *v.proto.borrow_mut() = built.map(|p| (key, p));
    }
    if let Some((_, proto)) = &*v.proto.borrow() {
        let sz = proto.size();
        let (w, h) = (sz.width.min(body.width), sz.height.min(body.height));
        let dest = Rect::new(body.x + (body.width - w) / 2, body.y + (body.height - h) / 2, w, h);
        v.last_rect.set(dest);
        v.placed.set(crate::render::graphics::place_protocol(proto, dest, buf));
    }
}

fn draw_external(v: &ExternalView, st: &Styles, area: Rect, buf: &mut Buffer, hits: &mut DocHits) {
    draw_title(&v.name, st, area, buf, hits);
    let x = area.x + 1;
    let mut y = area.y + 2;
    let w = area.width.saturating_sub(2).max(1) as usize;
    let line = |buf: &mut Buffer, y: &mut u16, text: &str, style: Style| {
        for l in wrap_lines(&[text.to_string()], w) {
            if *y < area.bottom() {
                put(buf, area, *y, x, &l, style);
            }
            *y += 1;
        }
    };
    if let Some(n) = &v.note {
        line(buf, &mut y, n, st.pager_notice);
    }
    if v.web {
        line(buf, &mut y, "This copy of lanthorn runs on a server, so there is no viewer to open it with. It is at:", st.pager);
        line(buf, &mut y, &v.path.to_string_lossy(), st.header);
    } else {
        line(buf, &mut y, "Opened in your system viewer.", st.pager);
        if y < area.bottom() {
            let url = crate::documents::file_url(&v.path);
            let rect = Rect::new(x, y, area.width.saturating_sub(1), 1);
            put_link(buf, rect, &v.path.display().to_string(), &url, st.header);
            hits.path_link = Some((rect, url));
            y += 2;
        }
        if y < area.bottom() {
            hits.open_viewer = Some(put(buf, area, y, x, OPEN_LABEL, st.button));
            y += 2;
        }
    }
    if y < area.bottom() {
        put(buf, area, y, x, "Esc returns to the list.", st.hint);
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn tab_with(files: &[(&str, &[u8])]) -> (DocumentsTab, PathBuf) {
        let dir = crate::scratch_dir("docs-tab");
        for (n, b) in files {
            std::fs::write(dir.join(n), b).unwrap();
        }
        let mut t = DocumentsTab::default();
        t.set_location(Location::Exists(dir.clone()));
        (t, dir)
    }

    fn render(state: &AppState, w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        draw(state, area, &mut buf);
        buf
    }

    fn text_of(buf: &Buffer) -> String {
        let a = buf.area;
        (a.y..a.bottom())
            .map(|y| (a.x..a.right()).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn state_with(tab: DocumentsTab) -> AppState {
        let mut s = AppState::default();
        s.journal_tab = crate::journal::JournalTab::Documents;
        s.documents_tab = tab;
        s
    }

    #[test]
    fn utf8_decodes_as_is_and_other_bytes_fall_back_to_cp437() {
        assert_eq!(decode_text("café ☺".as_bytes(), false), "café ☺");
        assert_eq!(decode_text(&[b'a', 0x82, 0xC4, 0xB3], false), "aé─│");
        // A prefix cut mid-sequence stays UTF-8 when it is known to be a prefix…
        let cut = &"é".as_bytes()[..1];
        assert_eq!(decode_text(&[b'x', cut[0]], true), "x");
        // …and is CP437 when it is the whole file.
        assert_eq!(decode_text(&[b'x', cut[0]], false), "x├");
    }

    #[test]
    fn cr_only_and_crlf_files_split_into_lines_and_controls_are_dropped() {
        assert_eq!(text_lines("one\rtwo\r\nthree\n"), ["one", "two", "three"]);
        assert_eq!(text_lines("a\tb\x1b[31m"), ["a       b[31m"]);
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_splits_long_words() {
        let w = |s: &str, n| wrap_lines(&[s.to_string()], n);
        assert_eq!(w("the quick brown fox", 10), ["the quick", "brown fox"]);
        assert_eq!(w("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(w("  indented words here", 12), ["  indented", "words here"]);
        assert_eq!(w("", 5), [""]);
    }

    #[test]
    fn pager_rewraps_on_resize_and_scrolls_within_bounds() {
        let body = "word ".repeat(60);
        let p = Pager::new("t.txt", body.as_bytes(), false);
        let narrow = p.wrapped_for(20).len();
        let wide = p.wrapped_for(60).len();
        assert!(narrow > wide, "{narrow} rows at 20 columns, {wide} at 60");
        p.max_scroll.set(3);
        p.scroll_by(10);
        assert_eq!(p.scroll.get(), 3, "clamped at the end");
        p.scroll_by(-10);
        assert_eq!(p.scroll.get(), 0);
    }

    #[test]
    fn a_description_shows_as_a_second_line_and_clicks_still_hit_the_right_row() {
        let (t, dir) = tab_with(&[]);
        let src = crate::scratch_dir("docs-tab-desc-src");
        for (n, d) in [("a.txt", Some("The first map")), ("b.txt", None), ("c.txt", Some("Third, with a very long description indeed"))] {
            std::fs::write(src.join(n), n.as_bytes()).unwrap();
            let meta = crate::documents::DocMeta::now(None, d.map(String::from), None, false);
            crate::documents::import_with(&dir, &src.join(n), Some(meta)).unwrap();
        }
        let mut t = t;
        t.set_location(Location::Exists(dir));
        let s = state_with(t);
        let buf = render(&s, 40, 14);
        let out = text_of(&buf);
        assert!(out.contains("The first map"), "{out}");
        let rows = s.documents_tab.hits().rows;
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|(_, r)| r.height == 2), "two-line rows: {rows:?}");
        assert!(rows.windows(2).all(|w| w[1].1.y == w[0].1.bottom()), "rows tile: {rows:?}");
        let line = |y: u16| (0..40).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(rows[0].1.y + 1).contains("The first map"));
        assert!(line(rows[2].1.y + 1).contains('…'), "truncated to the width: {}", line(rows[2].1.y + 1));
        let ev = |y| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 5, row: y, modifiers: crossterm::event::KeyModifiers::NONE };
        for (i, (idx, r)) in rows.iter().enumerate() {
            for y in [r.y, r.y + 1] {
                let hit = mouse_action(&s, &ev(y));
                assert!(matches!(hit, Some(DocMouse::Action(Action::DocTabClickRow(n))) if n == *idx), "row {i} line {y}");
            }
        }
        // Few lines: scrolling still works in whole rows.
        let small = text_of(&render(&s, 40, 9));
        assert!(small.contains("a.txt"), "{small}");
    }

    #[test]
    fn list_shows_kinds_sizes_and_a_spoiler_marker() {
        let (t, _d) = tab_with(&[("manual.txt", b"hello world\n"), ("walkthrough.txt", b"go north\n"), ("map.png", b"\x89PNG\r\n\x1a\nxx")]);
        let s = state_with(t);
        let out = text_of(&render(&s, 60, 14));
        assert!(out.contains("manual.txt") && out.contains("text"), "{out}");
        assert!(out.contains("map.png") && out.contains("image"), "{out}");
        assert!(out.contains("12 B"), "size shown: {out}");
        let flagged: Vec<&str> = out.lines().filter(|l| l.contains("spoiler")).collect();
        assert_eq!(flagged.len(), 1, "{out}");
        assert!(flagged[0].contains("walkthrough.txt"));
        assert!(out.contains("Download documents"), "{out}");
        assert!(!out.contains("Create documents folder"), "folder exists: {out}");
    }

    #[test]
    fn the_two_header_buttons_have_an_unstyled_gap_between_them() {
        let mut t = DocumentsTab::default();
        t.set_location(Location::Missing(PathBuf::from("/lib/documents/Zork [x]")));
        let s = state_with(t);
        let buf = render(&s, 70, 10);
        let h = s.documents_tab.hits();
        let (c, d) = (h.create.unwrap(), h.download.unwrap());
        assert_eq!(c.y, d.y, "both fit on one row at 70 columns");
        assert!(d.x > c.right(), "a gap cell between the hit-rects: {c:?} {d:?}");
        let gap = &buf[(c.right(), c.y)];
        assert_ne!(gap.style(), buf[(c.x, c.y)].style(), "the gap is not button-styled");
        assert_ne!(gap.style(), buf[(d.x, d.y)].style(), "the gap is not button-styled");
    }

    #[test]
    fn narrow_header_buttons_wrap_instead_of_overlapping() {
        let mut t = DocumentsTab::default();
        t.set_location(Location::Missing(PathBuf::from("/lib/documents/Zork [x]")));
        let s = state_with(t);
        let _ = render(&s, 40, 10);
        let h = s.documents_tab.hits();
        let (c, d) = (h.create.unwrap(), h.download.unwrap());
        assert!(d.y > c.y, "the second button wraps to its own row: {c:?} {d:?}");
        assert!(d.right() <= 40 && c.right() <= 40);
    }

    #[test]
    fn missing_unlinked_and_empty_say_so() {
        let mut t = DocumentsTab::default();
        t.set_location(Location::Missing(PathBuf::from("/lib/documents/Zork [x]")));
        let s = state_with(t);
        let out = text_of(&render(&s, 70, 10));
        assert!(out.contains("Create documents folder") && out.contains("Download documents"), "{out}");
        assert!(out.contains("not created"), "{out}");
        assert!(s.documents_tab.hits().create.is_some());

        let s = state_with(DocumentsTab::default());
        let out = text_of(&render(&s, 70, 10));
        assert!(out.contains(UNLINKED_LINE), "{out}");
        assert!(s.documents_tab.hits().download.is_none());

        let (t, _d) = tab_with(&[]);
        let s = state_with(t);
        let out = text_of(&render(&s, 70, 10));
        assert!(out.contains("Drop files into the folder"), "{out}");
        assert!(out.contains("from IFDB"), "{out}");
    }

    #[test]
    fn the_folder_header_is_an_osc8_link() {
        let (t, dir) = tab_with(&[("a.txt", b"x")]);
        let s = state_with(t);
        let buf = render(&s, 70, 8);
        let first: String = (0..70).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(first.contains("\x1b]8;;file://"), "{first:?}");
        assert!(s.documents_tab.hits().path_link.unwrap().1.starts_with("file://"));
        let _ = dir;
    }

    #[test]
    fn the_list_refreshes_after_an_import() {
        let (mut t, dir) = tab_with(&[("a.txt", b"x")]);
        assert_eq!(t.entries.len(), 1);
        let src = crate::scratch_dir("docs-tab-src").join("b.txt");
        std::fs::write(&src, b"y").unwrap();
        crate::documents::import(&dir, &src).unwrap();
        assert_eq!(t.entries.len(), 1, "no polling: nothing changes until told");
        t.mark_dirty();
        t.set_location(Location::Exists(dir));
        assert_eq!(t.entries.len(), 2);
        assert!(!t.dirty);
    }

    #[test]
    fn selection_survives_a_refresh_by_file_name() {
        let (mut t, dir) = tab_with(&[("a.txt", b"x"), ("c.txt", b"x")]);
        t.select(1);
        std::fs::write(dir.join("b.txt"), b"x").unwrap();
        t.set_location(Location::Exists(dir));
        assert_eq!(t.entries[t.selected].id, "c.txt");
    }

    #[test]
    fn opening_text_shows_the_pager_and_a_big_file_is_truncated() {
        let big = vec![b'a'; (TEXT_CAP + 10) as usize];
        let (mut t, _d) = tab_with(&[("a.txt", b"line one\rline two\r"), ("big.txt", &big)]);
        let mut spawned = Vec::new();
        assert_eq!(t.open_entry(0, false, false, &mut |p| spawned.push(p.to_string())), OpenOutcome::Opened);
        let DocView::Text(p) = &t.view else { panic!("pager") };
        assert_eq!(p.raw, ["line one", "line two"]);
        assert!(!p.truncated);
        assert!(spawned.is_empty());
        t.close_viewer();
        assert!(matches!(t.view, DocView::List));
        t.open_entry(1, false, false, &mut |_| {});
        let DocView::Text(p) = &t.view else { panic!("pager") };
        assert!(p.truncated);
        let s = state_with(t);
        let out = text_of(&render(&s, 40, 10));
        assert!(out.contains("only the first 4 MB"), "{out}");
    }

    #[test]
    fn a_pdf_goes_to_the_opener_and_web_mode_only_shows_the_path() {
        let (mut t, _d) = tab_with(&[("manual.pdf", b"%PDF-1.4 stuff")]);
        let mut spawned = Vec::new();
        assert_eq!(t.open_entry(0, false, false, &mut |p| spawned.push(p.to_string())), OpenOutcome::Opened);
        assert_eq!(spawned.len(), 1);
        assert!(spawned[0].ends_with("manual.pdf"));
        let s = state_with(t);
        let out = text_of(&render(&s, 70, 12));
        assert!(out.contains("Open in system viewer"), "{out}");
        assert!(s.documents_tab.hits().open_viewer.is_some());

        let (mut t, _d) = tab_with(&[("manual.pdf", b"%PDF-1.4 stuff")]);
        let mut spawned = Vec::new();
        t.open_entry(0, false, true, &mut |p| spawned.push(p.to_string()));
        assert!(spawned.is_empty(), "web mode never spawns");
        let s = state_with(t);
        let out = text_of(&render(&s, 90, 12));
        assert!(out.contains("manual.pdf") && out.contains("no viewer"), "{out}");
        assert!(s.documents_tab.hits().open_viewer.is_none());
    }

    #[test]
    fn a_spoiler_asks_first_and_the_answer_decides() {
        let (t, _d) = tab_with(&[("walkthrough.txt", b"go north\n")]);
        let mut s = state_with(t);
        open_selected(&mut s, None, false);
        assert!(matches!(s.documents_tab.view, DocView::List), "not opened yet");
        assert_eq!(s.overlays.confirm_spoiler_document.as_deref(), Some("walkthrough.txt"));
        assert!(s.any_modal_overlay_open());
        answer_spoiler(&mut s, false);
        assert!(matches!(s.documents_tab.view, DocView::List));
        assert!(s.overlays.confirm_spoiler_document.is_none());
        open_selected(&mut s, None, false);
        answer_spoiler(&mut s, true);
        assert!(matches!(s.documents_tab.view, DocView::Text(_)));
    }

    #[test]
    fn an_image_renders_through_the_picker_into_the_body() {
        let dir = crate::scratch_dir("docs-tab-img");
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255])))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("map.png"), &png).unwrap();
        let mut t = DocumentsTab::default();
        t.set_location(Location::Exists(dir));
        assert_eq!(t.open_entry(0, false, false, &mut |_| {}), OpenOutcome::Opened);
        assert!(matches!(t.view, DocView::Image(_)));
        let mut s = state_with(t);
        s.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let _ = render(&s, 40, 12);
        let DocView::Image(v) = &s.documents_tab.view else { panic!("image") };
        let r = v.last_rect.get();
        assert!(r.width > 0 && r.height > 0, "placed somewhere: {r:?}");
        assert!(r.y >= 1 && r.right() <= 40 && r.bottom() <= 12, "inside the body: {r:?}");
        // Closing returns to the list.
        close_viewer(&mut s);
        assert!(matches!(s.documents_tab.view, DocView::List));
    }

    #[test]
    fn a_double_click_opens_and_a_single_click_selects() {
        let (mut t, _d) = tab_with(&[("a.txt", b"x"), ("b.txt", b"x")]);
        let t0 = Instant::now();
        assert!(!t.click_row(1, t0));
        assert_eq!(t.selected, 1);
        assert!(t.click_row(1, t0 + Duration::from_millis(100)));
        assert!(!t.click_row(0, t0 + Duration::from_millis(900)));
        assert!(!t.click_row(0, t0 + Duration::from_millis(2000)), "too slow to be a double click");
    }

    #[test]
    fn mouse_routes_buttons_rows_and_the_wheel_and_leaves_other_tabs_alone() {
        use crossterm::event::KeyModifiers;
        let (t, _d) = tab_with(&[("a.txt", b"x")]);
        let s = state_with(t);
        let _ = render(&s, 60, 10);
        let h = s.documents_tab.hits();
        let ev = |kind, x, y| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
        let dl = h.download.unwrap();
        assert!(matches!(
            mouse_action(&s, &ev(MouseEventKind::Down(MouseButton::Left), dl.x + 1, dl.y)),
            Some(DocMouse::Command(CMD_DOWNLOAD))
        ));
        let row = h.rows[0].1;
        assert!(matches!(
            mouse_action(&s, &ev(MouseEventKind::Down(MouseButton::Left), row.x + 2, row.y)),
            Some(DocMouse::Action(Action::DocTabClickRow(0)))
        ));
        assert!(matches!(
            mouse_action(&s, &ev(MouseEventKind::ScrollDown, row.x, row.y)),
            Some(DocMouse::Action(Action::DocTabScroll(1)))
        ));
        assert!(mouse_action(&s, &ev(MouseEventKind::Down(MouseButton::Left), 200, 200)).is_none());
        let mut other = state_with(DocumentsTab::default());
        other.journal_tab = crate::journal::JournalTab::Map;
        assert!(mouse_action(&other, &ev(MouseEventKind::Down(MouseButton::Left), 1, 1)).is_none());
    }

    #[test]
    fn rows_use_the_selected_style_for_the_selection_only_in_both_colour_modes() {
        for honor in [true, false] {
            let (t, _d) = tab_with(&[("a.txt", b"x"), ("b.txt", b"x")]);
            let mut s = state_with(t);
            s.config.honor_game_colours = honor;
            let buf = render(&s, 40, 10);
            let rows = s.documents_tab.hits().rows;
            let sel = s.colors.theme.get("journal.docs.row:selected").style;
            let plain = s.colors.theme.get("journal.docs.row").style;
            let cell = |r: Rect| buf[(r.x + 3, r.y)].style();
            assert_eq!(cell(rows[0].1).fg, sel.fg, "honor_game_colours={honor}");
            assert_eq!(cell(rows[0].1).add_modifier, sel.add_modifier, "selected row reverses (honor={honor})");
            assert_eq!(cell(rows[1].1).fg, plain.fg, "honor_game_colours={honor}");
        }
    }

    #[test]
    fn every_documents_element_is_a_styleable_selector() {
        for name in [
            "journal.docs.row", "journal.docs.row:selected", "journal.docs.meta", "journal.docs.desc", "journal.docs.spoiler",
            "journal.docs.header", "journal.docs.button", "journal.docs.hint", "journal.docs.pager",
            "journal.docs.pager_title", "journal.docs.pager_notice",
        ] {
            assert!(crate::theme::registry::REGISTRY.iter().any(|r| r.name == name), "{name} is in the registry");
        }
    }

    fn key(code: crossterm::event::KeyCode, mods: crossterm::event::KeyModifiers) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, mods)
    }

    #[test]
    fn keys_drive_the_tab_without_taking_typing_or_plain_arrows() {
        use crate::input::key_to_action;
        use crossterm::event::{KeyCode, KeyModifiers as M};
        let (t, _d) = tab_with(&[("a.txt", b"hello"), ("b.txt", b"x")]);
        let mut s = state_with(t);
        let act = |s: &AppState, c, m| key_to_action(s, key(c, m));
        assert!(matches!(act(&s, KeyCode::Down, M::SHIFT), Action::DocTabSelect(1)));
        assert!(matches!(act(&s, KeyCode::Up, M::SHIFT), Action::DocTabSelect(-1)));
        assert!(matches!(act(&s, KeyCode::Right, M::SHIFT), Action::DocTabOpen(None)));
        // Plain typing, plain arrows and Esc are still the command line's.
        assert!(matches!(act(&s, KeyCode::Char('d'), M::NONE), Action::InputChar('d')));
        assert!(matches!(act(&s, KeyCode::Char('D'), M::SHIFT), Action::InputChar('D')));
        assert!(matches!(act(&s, KeyCode::Down, M::NONE), Action::HistoryNext));
        assert!(matches!(act(&s, KeyCode::Up, M::NONE), Action::HistoryPrev));
        assert!(matches!(act(&s, KeyCode::Left, M::NONE), Action::CursorLeft));
        assert!(!matches!(act(&s, KeyCode::Esc, M::NONE), Action::DocTabClose), "Esc on the list is not ours");
        // With a document open, Shift+Up/Down scroll it and Esc / Shift+Left close it.
        s.documents_tab.open_entry(0, false, false, &mut |_| {});
        assert!(s.documents_tab.viewer_open());
        assert!(matches!(act(&s, KeyCode::Down, M::SHIFT), Action::DocTabScroll(1)));
        assert!(matches!(act(&s, KeyCode::PageDown, M::SHIFT), Action::DocTabPage(1)));
        assert!(matches!(act(&s, KeyCode::Esc, M::NONE), Action::DocTabClose));
        assert!(matches!(act(&s, KeyCode::Left, M::SHIFT), Action::DocTabClose));
        assert!(matches!(act(&s, KeyCode::Char('d'), M::NONE), Action::InputChar('d')), "typing still types");
        // On any other tab the same keys keep their old meaning.
        s.journal_tab = crate::journal::JournalTab::Map;
        assert!(matches!(act(&s, KeyCode::Down, M::SHIFT), Action::Pan(0, 1)));
        assert!(!matches!(act(&s, KeyCode::Esc, M::NONE), Action::DocTabClose));
    }

    #[test]
    fn the_registry_commands_reach_the_tab() {
        use crate::keymap::Context;
        use crate::slash::{parse_in_context, SlashOutcome};
        let p = |l: &str| parse_in_context(l, '/', Context::Global);
        assert!(matches!(p("open-document"), SlashOutcome::Action(Action::DocTabOpen(None))));
        assert!(matches!(p("close-document"), SlashOutcome::Action(Action::DocTabClose)));
        assert!(matches!(p("select-document -2"), SlashOutcome::Action(Action::DocTabSelect(-2))));
        assert!(matches!(p("scroll-document 5"), SlashOutcome::Action(Action::DocTabScroll(5))));
        assert!(matches!(p("scroll-document page-down"), SlashOutcome::Action(Action::DocTabPage(1))));
        assert!(matches!(p("scroll-document sideways"), SlashOutcome::Error(_)));
        assert!(matches!(p(CMD_DOWNLOAD), SlashOutcome::DownloadDocuments));
        assert!(matches!(p(CMD_CREATE), SlashOutcome::CreateDocumentsFolder));
    }

    #[test]
    fn actions_open_scroll_and_close_through_apply_action() {
        use crate::input::apply_action;
        use mapper::mapper::Mapper;
        let long = "line\n".repeat(200);
        let (t, _d) = tab_with(&[("a.txt", long.as_bytes())]);
        let mut s = state_with(t);
        let mut m = Mapper::default();
        apply_action(Action::DocTabOpen(None), &mut s, &mut m);
        assert!(matches!(s.documents_tab.view, DocView::Text(_)));
        let _ = render(&s, 40, 10);
        apply_action(Action::DocTabScroll(5), &mut s, &mut m);
        let DocView::Text(p) = &s.documents_tab.view else { panic!("pager") };
        assert_eq!(p.scroll.get(), 5);
        apply_action(Action::DocTabPage(1), &mut s, &mut m);
        let DocView::Text(p) = &s.documents_tab.view else { panic!("pager") };
        assert_eq!(p.scroll.get(), 5 + 8, "a page is the viewport less one line (9 body rows here)");
        apply_action(Action::DocTabClose, &mut s, &mut m);
        assert!(matches!(s.documents_tab.view, DocView::List));
    }

    #[test]
    fn showing_the_tab_marks_it_stale_and_unlinked_without_a_library_says_so() {
        let mut s = AppState::default();
        s.documents_tab.dirty = false;
        s.set_journal_tab(crate::journal::JournalTab::Documents);
        assert!(s.documents_tab.dirty, "read the folder when the tab is shown");
        assert!(refresh_if_needed(&mut s, Path::new("/nowhere/story.z5")));
        assert!(!s.documents_tab.dirty);
        assert!(!refresh_if_needed(&mut s, Path::new("/nowhere/story.z5")), "nothing to do until told");
        let out = text_of(&render(&s, 50, 6));
        assert!(out.contains("No library"), "{out}");
    }

    fn zcode_hint() -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = 5;
        b
    }

    /// SQ-1690: a hint program is its own kind of row, labelled for what opening it
    /// does, and opening it hands back the path rather than paging the file.
    #[test]
    fn a_hint_program_row_says_so_and_opens_the_hints_tab_instead_of_a_pager() {
        let z = zcode_hint();
        let (t, dir) = tab_with(&[("zork1inv.z5", &z), ("notes.txt", b"hello")]);
        let hint = t.entries.iter().position(|e| e.id == "zork1inv.z5").unwrap();
        assert_eq!(t.entries[hint].kind, DocKind::HintProgram);
        let mut s = state_with(t);
        let shown = text_of(&render(&s, 70, 12));
        assert!(shown.contains("hint program \u{2014} opens in Hints tab"), "{shown}");
        // Even a spoiler-flagged hint file is not asked about: the Hints tab shows
        // nothing until the player asks it something.
        s.documents_tab.entries[hint].spoiler = true;
        let out = s.documents_tab.open_entry(hint, false, false, &mut |_| panic!("no external opener"));
        assert_eq!(out, OpenOutcome::HintProgram(dir.join("zork1inv.z5")));
        assert!(!s.documents_tab.viewer_open(), "no pager, no card");
    }

    #[test]
    fn opening_a_hint_program_shows_the_hints_tab_without_moving_focus() {
        let z = zcode_hint();
        let (t, dir) = tab_with(&[("zork1inv.z5", &z)]);
        let mut s = state_with(t);
        s.set_journal_tab(crate::journal::JournalTab::Documents);
        assert_eq!(s.focus, crate::state::Focus::Game);
        open_selected(&mut s, Some(0), false);
        assert_eq!(s.journal_tab, crate::journal::JournalTab::Hints);
        assert_eq!(s.focus, crate::state::Focus::Game, "the keyboard stays in the story");
        assert_eq!(s.hints_tab.phase, crate::hints_tab::Phase::NotStarted, "the run loop starts the session");
        // The loop's next turn starts it and remembers this file as the game's hints.
        s.config.user_dir = crate::scratch_dir("docs-tab-hint-user");
        let story = dir.join("story.z3");
        std::fs::write(&story, b"story").unwrap();
        crate::hints_tab::ensure_started_in(&mut s, &story, "IFID-X", Some(&dir));
        assert_ne!(s.hints_tab.phase, crate::hints_tab::Phase::NotStarted);
        assert_eq!(crate::hints::load_hint_index(&s.config.user_dir).get("IFID-X"), Some(dir.join("zork1inv.z5")));
    }
}
