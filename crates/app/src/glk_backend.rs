//! The app's [`gvm::glk::GlkBackend`] implementation ([`AppGlk`]).
//!
//! A running Glulx game drives Glk display calls (window open/close/arrange,
//! `put_text`, `grid_put`/`grid_clear`, …); `AppGlk` records them and projects
//! them onto the engine-neutral [`ScreenModel`] window tree (the same tree the
//! Z-machine produces), so the one generic renderer draws both engines.
//!
//! Glk styles map to the same text-style bits the transcript runs use
//! ([`glk_style_bits`]), so emphasis renders for free. The **primary** text-
//! buffer window (the first one opened) is the one whose output the app mirrors
//! into `state.transcript` (search / persistence / styling); its new text is
//! drained via [`AppGlk::take_transcript`]. Extra buffer windows carry their
//! inline content in the [`BufferWindow`] node.

use std::any::Any;
use std::collections::BTreeMap;

use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, StyleAttrs, StyleColour, WinTree, WinType};

use crate::engine::{
    BorderPref, BufferWindow, GridCell, GridWindow, ScreenModel, Split, StatusModel, WinNode,
};
use crate::state::StyleRun;

// ── Glk style → text-style bits ────────────────────────────────────────────────

/// Map a Glk style class to the neutral text-style bitset used by the transcript
/// runs (1 = reverse, 2 = bold, 4 = italic, 8 = fixed-pitch).
pub fn glk_style_bits(style: GlkStyle) -> u8 {
    match style {
        GlkStyle::Emphasized => 0x04,   // italic (Glk emphasis, matching Gargoyle)
        GlkStyle::Header => 0x02,       // bold
        GlkStyle::Subheader => 0x02,    // bold
        GlkStyle::Input => 0x02,        // bold
        GlkStyle::Alert => 0x03,        // bold + reverse
        GlkStyle::Preformatted => 0x08, // fixed-pitch
        GlkStyle::Normal
        | GlkStyle::Note
        | GlkStyle::BlockQuote
        | GlkStyle::User1
        | GlkStyle::User2 => 0,
    }
}

/// Resolve a Glk style class + its stylehint colour and rendered attribute
/// hints into the app's neutral `(style-bits, packed-fg, packed-bg)`. The
/// reverse hint sets bit `0x01`; 24-bit RGB is carried losslessly via
/// [`ZColour::True24`](zvm::screen::ZColour::True24). Packed colours use
/// [`crate::state::pack_zcolour`] (`0` = `ZColour::Default`).
///
/// The Weight/Oblique stylehints layer on top of the class's intrinsic bits
/// (SQ-0317): a set hint overrides (Weight 1 → bold on, 0 → off; a "lighter"
/// weight has no terminal rendering, so any non-1 value clears bold; Oblique
/// 1 → italic on, other → off), an unset hint keeps the class default.
///
/// Colour is recorded unconditionally — the `honor_game_colours` gate is applied
/// at *render* time by `cell_style`/`draw_str_runs`, exactly like the Z-machine,
/// so toggling it (F2) recolours already-drawn output too.
fn resolve_glk_colour(style: GlkStyle, colour: StyleColour, attrs: StyleAttrs) -> (u8, u32, u32) {
    let mut bits = glk_style_bits(style);
    match attrs.weight {
        Some(1) => bits |= 0x02,
        Some(_) => bits &= !0x02,
        None => {}
    }
    match attrs.oblique {
        Some(1) => bits |= 0x04,
        Some(_) => bits &= !0x04,
        None => {}
    }
    if colour.reverse {
        bits |= 0x01;
    }
    let pack = |o: Option<u32>| {
        o.map(|v| crate::state::pack_zcolour(zvm::screen::ZColour::True24(v))).unwrap_or(0)
    };
    (bits, pack(colour.fg), pack(colour.bg))
}

/// Resolve the paragraph layout stylehints (`stylehint_Indentation`,
/// `stylehint_ParaIndentation`, `stylehint_Justification`) recorded on a run's
/// [`StyleAttrs`] into the app's neutral [`crate::state::ParaFmt`] (SQ-0330):
/// indent clamped to `[0, u16::MAX]` cells, para_indent clamped to an `i16`
/// (negative = hanging first line), justify clamped to `0..=3` (unknown → left).
/// An unset hint defaults to 0 (left, no indent), so a buffer that set no layout
/// hints — and the whole Z-machine path — renders exactly as before.
fn resolve_glk_para(attrs: StyleAttrs) -> crate::state::ParaFmt {
    crate::state::ParaFmt {
        // Glk has no `buffer_mode`; its text windows always word-wrap.
        nowrap_from: None,
        indent: attrs.indent.unwrap_or(0).clamp(0, u16::MAX as i32) as u16,
        para_indent: attrs.para_indent.unwrap_or(0).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        justify: attrs.justify.unwrap_or(0).min(3) as u8,
    }
}

/// One `(fg, bg)` theme colour pair reported through `glk_style_measure`
/// (SQ-0315): each channel `Some(0x00RRGGBB)` or `None` for terminal-default.
pub type ThemePair = (Option<u32>, Option<u32>);

/// The theme's rendered default colours for every Glk style class, by window
/// type (SQ-0803): row 0 = text-buffer windows, row 1 = text-grid windows;
/// the index is the Glk style class (0=Normal .. 10=User2, `style_NUMSTYLES`
/// = 11 per glk.h). Slot 0 (Normal) IS the window's element base — per SQ-0331
/// the Normal slot is definitionally the element — so a theme with no per-style
/// colours reports the same pair in all eleven entries of a row.
pub type GlkStylePairs = [[ThemePair; 11]; 2];

/// The theme's rendered default colours for Glk windows, derived from the
/// active [`ColorScheme`](crate::colors::ColorScheme): a per-style-class pair
/// for text-buffer (row 0, base `transcript` — the story pane) and text-grid
/// (row 1, base `status_bar`) windows. Only concrete RGB colours are reported;
/// a named/indexed ANSI colour or an unset channel renders however the terminal
/// decides, so it is honestly `None` (no guess).
///
/// Each style resolves exactly the way the renderer resolves it (SQ-0331,
/// `render::resolve_glk_channel` with no game-set colour): the theme's
/// `glk_styles[row][style]` slot, else the element base. The slot applies in
/// BOTH `honor_game_colours` modes, so this needs no gate. A discovered
/// `garglk.ini` populates those slots (`GarglkOverlay::apply`), which is how
/// Kerkerkruip's `style_User2` = `0xF400A1` sentinel gets a truthful answer
/// (SQ-0803): **shipping the ini beside the story IS the opt-in** — we honour
/// what the author's config says we paint, and a player who does not want the
/// game's Gargoyle-flavoured presentation simply does not keep the ini there.
///
/// `transcript` reads the legacy field (SQ-0309: kept — see its doc comment on
/// `ColorScheme`), not `colors.theme`: this is called from `startup.rs` before
/// `colors.theme` is rebuilt with the layered global/garglk/per-game decls, so
/// only the legacy field (which a discovered `garglk.ini` overlay patches
/// directly) is current at that point. `status_bar` has no such hazard — it is
/// read through the theme.
pub fn theme_style_colours(colors: &crate::colors::ColorScheme) -> GlkStylePairs {
    let rgb = |c: Option<ratatui::style::Color>| match c {
        Some(ratatui::style::Color::Rgb(r, g, b)) => {
            Some(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
        }
        _ => None,
    };
    let base: [ThemePair; 2] = [
        (rgb(colors.transcript.fg), rgb(colors.transcript.bg)),
        (rgb(colors.theme.get("status_bar").style.fg), rgb(colors.theme.get("status_bar").style.bg)),
    ];
    // A slot channel the theme sets is what gets painted, so it is the answer —
    // but only when it is a concrete RGB; a named ANSI slot colour IS rendered
    // and is still unknowable, so it reports `None` rather than falling through
    // to a base colour the player never sees.
    let channel = |slot: Option<ratatui::style::Color>, base: Option<u32>| match slot {
        None => base,
        some => rgb(some),
    };
    let mut out: GlkStylePairs = [[(None, None); 11]; 2];
    for (row, pairs) in out.iter_mut().enumerate() {
        for (style, pair) in pairs.iter_mut().enumerate() {
            let slot = colors.glk_styles[row][style];
            *pair = (channel(slot.fg, base[row].0), channel(slot.bg, base[row].1));
        }
    }
    out
}

// ── Per-window record ──────────────────────────────────────────────────────────

/// One text-grid cell: `(char, style-bits, packed-fg, packed-bg, link, glk_style)`.
type GridBufCell = (char, u8, u32, u32, u32, u8);

/// A text-grid window's cell buffer (cells keyed by 0-based `(row, col)`).
#[derive(Default, Clone)]
struct GridBuf {
    width: u32,
    height: u32,
    /// `(row, col) -> (char, style-bits, packed-fg, packed-bg, link, glk_style)`.
    /// `link` is the Glk hyperlink value stamped on the cell (0 = not a link)
    /// (SQ-0258); `glk_style` is the Glk style class (0=Normal, SQ-0331).
    cells: BTreeMap<(u32, u32), GridBufCell>,
}

/// One entry in a text-buffer window's ordered output log.
#[derive(Clone)]
enum BufElem {
    /// A run of printed text with its style bits, packed colours, Glk hyperlink
    /// value (0 = no link), paragraph layout format (SQ-0330) and Glk style class
    /// (0=Normal .. 10=User2, for the theme's per-style colour slot, SQ-0331).
    Text { bits: u8, fg: u32, bg: u32, link: u32, para: crate::state::ParaFmt, glk_style: u8, text: String },
    /// An image drawn into this buffer window (Glk `glk_image_draw`).
    Image(crate::inline_image::InlineImage),
}

/// A text-buffer window's ordered output log (text runs + inline images).
#[derive(Default, Clone)]
struct BufBuf {
    log: Vec<BufElem>,
    /// Number of leading log entries already drained by `take_transcript*`.
    drained: usize,
    /// Scrollback offset for an inline (non-primary) buffer window.
    scroll: u16,
}

// ── The backend ────────────────────────────────────────────────────────────────

/// A live Glk sound channel's app-side state.
struct SoundChannel {
    rock: u32,
    /// Glk volume (0x10000 = full); snapshotted into each `Play` op.
    volume: u32,
    /// Whether the channel is paused (Glk 0.7.3 §8.3). Set by `schannel_pause`,
    /// cleared by `schannel_unpause`, and snapshotted into each `Play` op so a
    /// sound played on a channel paused while empty starts paused.
    paused: bool,
}

/// Everything a question asked behind the player's back must put back (SQ-1293).
///
/// `Machine::restore_state` rolls back *gvm's* Glk model, and none of this: the
/// backend keeps its own copies of what every window holds, and the app renders
/// from those, not from gvm. So a silent `look` whose VM is restored still leaves
/// its room description in `buffers[*].log` — where `screen_model` and
/// `window_dump_lines` both read the WHOLE log, not the undrained tail — and its
/// status-line rewrite in `grids[*].cells`, on a screen whose transcript never saw
/// either. Draining is not undoing: `take_transcript_elems` only advances
/// `drained`.
///
/// The buffer logs are cloned whole rather than remembered by length, because a
/// window the question CLEARS shrinks rather than grows and a length cannot undo
/// that. It is affordable because it is rare — see
/// [`crate::glulx_session::GlulxSession::silent_look`] for what limits how often a
/// question is asked at all.
///
/// `layout` and `layout_tree` are here too, and for the same reason they look
/// redundant: `refresh_screen`'s `sync_window_tree` re-pushes gvm's own tree after
/// the restore, so the tree ends up gvm's either way — but the flat `layout` has no
/// such re-push, and a story that reopens a window on `look` would otherwise leave
/// the hit-test rectangles describing a screen that never happened.
pub(crate) struct DisplaySnapshot {
    layout: Vec<(u32, WinType, GlkRect, Option<bool>)>,
    layout_tree: Option<WinTree>,
    grids: BTreeMap<u32, GridBuf>,
    buffers: BTreeMap<u32, BufBuf>,
    scans: BTreeMap<u32, StoryScan>,
    graphics: BTreeMap<u32, crate::graphics::Canvas>,
    primary: Option<u32>,
    primary_cleared: bool,
}

/// The app Glk display backend (see the module docs).
pub struct AppGlk {
    /// Reported display size (the story-pane size the game lays windows out in).
    cols: u32,
    rows: u32,
    /// The latest resolved leaf-window layout `(id, type, rect, border)`. The
    /// border hint is `None` (no preference), `Some(false)` (`winmethod_NoBorder`),
    /// or `Some(true)` (`winmethod_Border`). (SQ-0286)
    layout: Vec<(u32, WinType, GlkRect, Option<bool>)>,
    /// gvm's live window tree (position-ordered children, border hints), or
    /// `None` when no root window exists. Delivered by `window_tree`; walked by
    /// `screen_model` into the neutral `WinNode` tree. (The flat `layout` above
    /// still feeds `mouse_windows`/`hyperlink_windows` hit-testing.)
    layout_tree: Option<WinTree>,
    grids: BTreeMap<u32, GridBuf>,
    buffers: BTreeMap<u32, BufBuf>,
    /// The primary buffer window id (the first text-buffer opened), if any.
    primary: Option<u32>,
    /// Set when the primary buffer window is cleared (`glk_window_clear`) this
    /// turn — an Inform 7 menu redraw clears + reprints on every keypress. Taken
    /// by `finish_turn` into `TurnResult.erase_lower` so the app pins the reprint
    /// to a fresh screen instead of appending a fresh copy each time. (SQ-0403)
    primary_cleared: bool,
    /// The room-heading / read-prompt scan, **one per buffer window** (SQ-1241).
    ///
    /// It has to be per-window rather than per-`primary`, because which buffer
    /// is primary can change *after* the text that answers the question has
    /// already been written. City of Secrets (GWindows) prints its whole
    /// prologue — title, `Subheader` "City Train Station", room description and
    /// read prompt — into a second buffer it opens mid-turn, while `primary` is
    /// still the splash window it opened first; `set_input_window` only re-points
    /// primary at the end of that turn, from `finish_turn`. Scanning only the
    /// window that was primary AT WRITE TIME therefore missed the opening room
    /// heading outright, and judged the banner test against the splash window's
    /// read prompt (which there was none of) — so the story ran for four turns
    /// with no location at all. Every buffer is scanned as it is written and
    /// [`take_room_heading`](Self::take_room_heading) reads whichever scan
    /// belongs to the primary of the moment, so a window that becomes the story
    /// window brings its own history with it.
    scans: BTreeMap<u32, StoryScan>,
    /// Graphics-window pixel canvases, keyed by window id.
    graphics: std::collections::BTreeMap<u32, crate::graphics::Canvas>,
    /// The `(width, height)` of one text-grid cell in pixels, for pixel↔cell
    /// layout of graphics windows.
    char_px: (u32, u32),
    /// Resolves + caches Blorb `Pict` resources for `graphics_draw_image`.
    picts: crate::graphics::PictSource,
    /// Live sound channels, keyed by Glk channel ref (BTree for stable iterate).
    schannels: BTreeMap<u32, SoundChannel>,
    /// Next channel ref to hand out (pre-incremented; first create → 1).
    next_schannel: u32,
    /// Buffered per-turn sound operations, drained by `take_sound_ops`.
    sound_ops: Vec<crate::session::SchannelOp>,
    /// The theme's rendered default `(fg, bg)` per Glk style class, by window
    /// type, as `0x00RRGGBB`; `None` per channel = terminal default (unknowable
    /// — no guess). See [`GlkStylePairs`]. Reported to the game through
    /// `glk_style_measure` via `GlkBackend::default_style_colours` so it can
    /// detect a dark background (SQ-0315) or probe what we paint for one style
    /// class (SQ-0803). Pushed by the app on boot and kept fresh each loop pass
    /// (live style reload).
    theme_styles: GlkStylePairs,
}

impl Default for AppGlk {
    fn default() -> Self {
        AppGlk::new(80, 24)
    }
}

/// How much output after the blank line following a heading candidate is kept.
/// Only enough to recognise a bare read prompt; anything longer is prose.
const HEADING_TAIL_CAP: usize = 32;

/// How much of a window's trailing output is kept in
/// `StoryScan::prompt_tail`. Only enough to see a read prompt and the newline that
/// puts it at line start.
const PROMPT_TAIL_CAP: usize = 32;

/// How much of the candidate's OWN line, after the bold run ends, is kept in
/// `StoryScan::heading_line_rest`. Only enough to see whether a word follows —
/// see [`StoryScan::line_rest_disqualifies`].
const HEADING_LINE_REST_CAP: usize = 24;

/// Where a window's output stream sits relative to the heading
/// candidate held in `StoryScan::heading_pending` — the states of the "is this
/// heading joined to a room description, or set off as a banner?" test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HeadingTail {
    /// No candidate is awaiting a verdict.
    Idle,
    /// Still on the candidate's own line (a heading run can end before the line
    /// does — Inform prints "Kitchen" in `Subheader` and the "(on the chair)"
    /// parenthetical after it in roman).
    Line,
    /// The candidate's line just ended; the very next character decides.
    LineEnd,
    /// A blank line followed the candidate, so it stands apart from whatever
    /// comes next. `heading_tail_text` collects that "whatever".
    Detached,
}

/// One buffer window's room-heading and read-prompt scan (SQ-1241).
///
/// Every field here used to sit on [`AppGlk`] and be fed only for the window
/// that was `primary` at write time; see the doc on `AppGlk::scans` for why that
/// lost City of Secrets' opening room. The state machine itself is unchanged —
/// it simply now belongs to the window whose stream it describes.
#[derive(Clone)]
struct StoryScan {
    /// Accumulator for the current run of `Subheader` text (the Inform room
    /// heading, captured char-by-char).
    heading_acc: String,
    /// The last completed `Subheader` line seen since the previous drain — the
    /// current room heading (`None` if this turn printed none).
    last_heading: Option<String>,
    /// Whether this window's output stream is at the start of a line.
    /// A room heading is a `Subheader` run that BEGINS here; `Subheader` runs
    /// beginning mid-line are inline hyperlinks (e.g. Superluminal's command
    /// hints), not rooms.
    at_line_start: bool,
    /// Whether `heading_acc` is an active heading run (a `Subheader` run that
    /// began at line start and has not yet been terminated).
    in_heading: bool,
    /// A finished heading line still awaiting the verdict of what follows it —
    /// see [`HeadingTail`]. Promoted to `last_heading` once it is confirmed.
    heading_pending: Option<String>,
    /// Where the output stream sits relative to `heading_pending`.
    heading_tail: HeadingTail,
    /// Output seen since the blank line that detached `heading_pending` from the
    /// rest of the turn, capped at [`HEADING_TAIL_CAP`] chars.
    heading_tail_text: String,
    /// Set once `heading_tail_text` hit the cap: whatever follows the blank line
    /// is far too long to be a bare read prompt, so it is prose.
    heading_tail_prose: bool,
    /// What followed the bold run on the candidate's OWN line, capped at
    /// [`HEADING_LINE_REST_CAP`] chars — the evidence for
    /// [`StoryScan::line_rest_disqualifies`].
    heading_line_rest: String,
    /// A candidate that a `Subheader` run opening the line BELOW it displaced
    /// before anything could settle it — see [`StoryScan::capture_heading`]
    /// (SQ-1295). Confirmed by [`StoryScan::reject_heading`] when the line that
    /// displaced it turns out to be prose, dropped when that line turns out to
    /// own itself (another banner).
    heading_displaced: Option<String>,
    /// The last [`PROMPT_TAIL_CAP`] chars written to this window, kept only to
    /// answer "does the stream end at the game's read prompt?" — the test for a
    /// parser command prompt rather than a bare line read.
    prompt_tail: String,
}

impl Default for StoryScan {
    fn default() -> Self {
        StoryScan {
            heading_acc: String::new(),
            last_heading: None,
            at_line_start: true,
            in_heading: false,
            heading_pending: None,
            heading_tail: HeadingTail::Idle,
            heading_tail_text: String::new(),
            heading_tail_prose: false,
            heading_line_rest: String::new(),
            heading_displaced: None,
            prompt_tail: String::new(),
        }
    }
}

/// One styled text chunk drained by `take_transcript`: `(char_count, style-bits,
/// fg, bg, link, paragraph format, glk_style, nowrap)`. Glk has no `buffer_mode`
/// equivalent — its text windows always word-wrap — so `nowrap` is always
/// `false` on this path.
type TranscriptChunk = crate::session::CaptureRun;

impl AppGlk {
    /// A backend reporting a `cols × rows` display. Stylehint colour is always
    /// recorded; the `honor_game_colours` gate is applied at render time.
    pub fn new(cols: u32, rows: u32) -> AppGlk {
        AppGlk::with_graphics(cols, rows, (1, 1), crate::graphics::PictSource::new(None))
    }

    /// A backend also carrying the char-cell pixel size and a `Pict` source,
    /// needed for graphics windows.
    pub fn with_graphics(
        cols: u32,
        rows: u32,
        char_px: (u32, u32),
        picts: crate::graphics::PictSource,
    ) -> AppGlk {
        AppGlk {
            cols,
            rows,
            layout: Vec::new(),
            layout_tree: None,
            grids: BTreeMap::new(),
            buffers: BTreeMap::new(),
            primary: None,
            primary_cleared: false,
            scans: BTreeMap::new(),
            graphics: BTreeMap::new(),
            char_px,
            picts,
            schannels: BTreeMap::new(),
            next_schannel: 0,
            sound_ops: Vec::new(),
            theme_styles: [[(None, None); 11]; 2],
        }
    }

    /// Update the theme's rendered default colours reported through
    /// `glk_style_measure` (SQ-0315/SQ-0803): one `(fg, bg)` pair per Glk style
    /// class for text-buffer windows (row 0) and text-grid windows (row 1), as
    /// built by [`theme_style_colours`]. Each channel is `Some(0x00RRGGBB)` or
    /// `None` for a terminal-default (no explicit colour — honestly unknown).
    pub fn set_theme_colours(&mut self, styles: GlkStylePairs) {
        self.theme_styles = styles;
    }

    /// The pixel size of a graphics window `win` as laid out, from `layout` ×
    /// `char_px`. Does not borrow `self.graphics`, so it can be called while
    /// `self.graphics.entry(..)` is held.
    fn canvas_size(&self, win: u32) -> (u32, u32) {
        let cells = self
            .layout
            .iter()
            .find(|&&(id, _, _, _)| id == win)
            .map(|&(_, _, r, _)| (r.width, r.height))
            .unwrap_or((1, 1));
        (cells.0 * self.char_px.0, cells.1 * self.char_px.1)
    }

    /// Update the reported display size (the host story-pane size each frame).
    pub fn set_screen_size(&mut self, cols: u32, rows: u32) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
    }

    /// The primary text-buffer window id, if one is open.
    pub fn primary(&self) -> Option<u32> {
        self.primary
    }

    /// Point the primary buffer — the window whose text becomes the scrollback
    /// and over which the inline input prompt is drawn — at the window the game
    /// is taking line input on. Most games take input on their sole
    /// (first-opened) buffer, so this is a no-op; but a game whose real story +
    /// prompt live in a later-opened window (narco opens a decorative pane
    /// first, then does everything in its second buffer) would otherwise route
    /// the prompt and transcript to the wrong, near-empty window. `None` (a
    /// char-input turn or no pending request) and non-buffer/unknown windows
    /// leave the current primary untouched, so the common single-buffer and
    /// char-input paths are byte-identical. (SQ-0337)
    pub fn set_input_window(&mut self, win: Option<u32>) {
        if let Some(w) = win {
            if self.buffers.contains_key(&w) {
                self.primary = Some(w);
            }
        }
    }

    /// Format the live Glk window tree as indented diagnostic lines for the
    /// `/dump-windows` command: one window per line with its type, id, size,
    /// origin, and any per-window `bg`/`fg` colour; each pair shows orientation,
    /// border presence, split, and its key window's colour. (SQ-0329)
    pub fn window_dump_lines(&self) -> Vec<String> {
        let Some(tree) = &self.layout_tree else {
            return vec!["Window layout: (none)".to_string()];
        };
        let r = tree.rect();
        let mut out = vec![format!("Window layout ({}x{}):", r.width, r.height)];
        fn col(label: &str, c: Option<u32>) -> String {
            match c {
                Some(rgb) => format!(" {}=#{:06X}", label, rgb & 0x00FF_FFFF),
                None => String::new(),
            }
        }
        // Per-graphics-window canvas diagnostics: size + how many pixels the game
        // has actually painted (opaque). A window in the tree with `opaque=0` means
        // the game hasn't drawn it (or a resize cleared it) — the source of "black/
        // missing graphics". (SQ-0332)
        let gfx: std::collections::BTreeMap<u32, String> = self
            .graphics
            .iter()
            .map(|(id, c)| {
                let opaque = c.img.pixels().filter(|p| p.0[3] != 0).count();
                (*id, format!(" canvas={}x{} v{} opaque={}", c.img.width(), c.img.height(), c.version, opaque))
            })
            .collect();
        fn walk(node: &WinTree, depth: usize, primary: Option<u32>, gfx: &std::collections::BTreeMap<u32, String>, out: &mut Vec<String>) {
            let indent = "  ".repeat(depth);
            match node {
                WinTree::Leaf { id, wintype, rect, bg, fg, .. } => {
                    let ty = match wintype {
                        WinType::TextGrid => "Grid",
                        WinType::TextBuffer => "Buffer",
                        WinType::Graphics => "Graphics",
                        WinType::Pair => "Pair",
                    };
                    let prim = if primary == Some(*id) { " (primary)" } else { "" };
                    let ginfo = if *wintype == WinType::Graphics {
                        gfx.get(id).cloned().unwrap_or_else(|| " canvas=none".to_string())
                    } else {
                        String::new()
                    };
                    out.push(format!(
                        "{}{} id={}{}  {}x{} @({},{}){}{}{}",
                        indent, ty, id, prim, rect.width, rect.height, rect.left, rect.top,
                        col("bg", *bg), col("fg", *fg), ginfo,
                    ));
                }
                WinTree::Pair { vertical, border, split, key_bg, first, second, .. } => {
                    let orient = if *vertical { "vertical" } else { "horizontal" };
                    let brd = if *border { "border" } else { "no-border" };
                    out.push(format!(
                        "{}Pair  {}  {}  split={}{}",
                        indent, orient, brd, split, col("key", *key_bg),
                    ));
                    walk(first, depth + 1, primary, gfx, out);
                    walk(second, depth + 1, primary, gfx, out);
                }
            }
        }
        walk(tree, 0, self.primary, &gfx, &mut out);
        out
    }

    /// The current resolved leaf-window layout `(id, type, rect, border)`.
    /// Rects are in story-pane cells (the Glk screen is sized to exactly the
    /// story pane). The host reads this to map terminal clicks to mouse-watching
    /// windows. The border hint is `None`/`Some(false)`/`Some(true)` (SQ-0286).
    pub fn layout(&self) -> &[(u32, WinType, GlkRect, Option<bool>)] {
        &self.layout
    }

    /// Drain the primary window's text printed since the last drain, as
    /// `(text, (char_count, bits, fg, bg) chunks)` for `push_transcript_runs`.
    /// fg/bg carry the resolved stylehint colour (24-bit via `ZColour::True24`).
    pub fn take_transcript(&mut self) -> (String, Vec<TranscriptChunk>) {
        let Some(pid) = self.primary else {
            return (String::new(), Vec::new());
        };
        let Some(buf) = self.buffers.get_mut(&pid) else {
            return (String::new(), Vec::new());
        };
        let mut text = String::new();
        let mut chunks: Vec<TranscriptChunk> = Vec::new();
        for elem in &buf.log[buf.drained..] {
            let BufElem::Text { bits, fg, bg, link, para, glk_style, text: s } = elem else { continue };
            let n = s.chars().count();
            if n == 0 {
                continue;
            }
            chunks.push((n, *bits, crate::state::unpack_zcolour(*fg), crate::state::unpack_zcolour(*bg), *link, *para, *glk_style, false));
            text.push_str(s);
        }
        buf.drained = buf.log.len();
        (text, chunks)
    }

    /// Drain the primary window's undrained log into ordered transcript
    /// elements (consecutive text runs coalesced; images preserved in place).
    pub fn take_transcript_elems(&mut self) -> Vec<crate::session::TranscriptElem> {
        use crate::session::TranscriptElem;
        let Some(pid) = self.primary else { return Vec::new() };
        let Some(buf) = self.buffers.get_mut(&pid) else { return Vec::new() };
        let mut out: Vec<TranscriptElem> = Vec::new();
        // Accumulate consecutive Text runs into one element, matching the
        // char-count chunk shape `push_transcript_runs` expects:
        // (char_count, bits, fg, bg).
        let mut cur_text = String::new();
        let mut cur_runs: Vec<TranscriptChunk> = Vec::new();
        let flush = |out: &mut Vec<TranscriptElem>, text: &mut String, runs: &mut Vec<_>| {
            if !text.is_empty() {
                out.push(TranscriptElem::Text { text: std::mem::take(text), runs: std::mem::take(runs) });
            } else {
                runs.clear();
            }
        };
        for elem in &buf.log[buf.drained..] {
            match elem {
                BufElem::Text { bits, fg, bg, link, para, glk_style, text } => {
                    let n = text.chars().count();
                    if n > 0 {
                        // Convert packed u32 colours back to ZColour to match
                        // the chunk type push_transcript_runs consumes.
                        let (f, b) = (crate::state::unpack_zcolour(*fg), crate::state::unpack_zcolour(*bg));
                        cur_runs.push((n, *bits, f, b, *link, *para, *glk_style, false));
                        cur_text.push_str(text);
                    }
                }
                BufElem::Image(img) => {
                    flush(&mut out, &mut cur_text, &mut cur_runs);
                    out.push(TranscriptElem::Image(img.clone()));
                }
            }
        }
        flush(&mut out, &mut cur_text, &mut cur_runs);
        buf.drained = buf.log.len();
        out
    }

    /// Drain the sound operations buffered this turn (see [`crate::session::SchannelOp`]).
    pub fn take_sound_ops(&mut self) -> Vec<crate::session::SchannelOp> {
        std::mem::take(&mut self.sound_ops)
    }

    /// Take (and reset) the "primary buffer cleared this turn" flag — a
    /// `glk_window_clear` on the primary window, e.g. an Inform 7 menu redraw.
    /// Fed into `TurnResult.erase_lower` so the reprint replaces the screen
    /// instead of stacking a fresh copy. (SQ-0403)
    pub fn take_primary_cleared(&mut self) -> bool {
        std::mem::take(&mut self.primary_cleared)
    }

    /// Test seam: append an inline image to the primary buffer's undrained log,
    /// as a game's `glk_image_draw` into the buffer window would (a resolvable
    /// Pict needs a Blorb the unit harness lacks). Lets a `GlulxSession` test
    /// exercise the banner/startup image path without a Blorb.
    #[cfg(all(test, feature = "t-session"))]
    pub(crate) fn test_push_primary_image(&mut self, img: crate::inline_image::InlineImage) {
        if let Some(pid) = self.primary {
            if let Some(buf) = self.buffers.get_mut(&pid) {
                buf.log.push(BufElem::Image(img));
            }
        }
    }

    /// The scan belonging to one buffer window, created on first write.
    fn scan(&mut self, win: u32) -> &mut StoryScan {
        self.scans.entry(win).or_default()
    }

    /// The scan of the window that is primary *right now* — the story window as
    /// of this moment, which is not necessarily the one the text was written to
    /// (see `Self::scans`).
    fn primary_scan(&mut self) -> Option<&mut StoryScan> {
        let pid = self.primary?;
        Some(self.scans.entry(pid).or_default())
    }

    /// Take a [`DisplaySnapshot`] of everything a silent question could disturb.
    /// Paired with [`Self::restore_display_snapshot`]; see the snapshot's own docs
    /// for what is in it and why (SQ-1293).
    pub(crate) fn display_snapshot(&self) -> DisplaySnapshot {
        DisplaySnapshot {
            layout: self.layout.clone(),
            layout_tree: self.layout_tree.clone(),
            grids: self.grids.clone(),
            buffers: self.buffers.clone(),
            scans: self.scans.clone(),
            graphics: self.graphics.clone(),
            primary: self.primary,
            primary_cleared: self.primary_cleared,
        }
    }

    /// Put a [`DisplaySnapshot`] back, wholesale. Windows the question opened go
    /// with it, because the map is replaced rather than merged — and gvm's own
    /// model, restored alongside, does not have them either.
    pub(crate) fn restore_display_snapshot(&mut self, snap: DisplaySnapshot) {
        self.layout = snap.layout;
        self.layout_tree = snap.layout_tree;
        self.grids = snap.grids;
        self.buffers = snap.buffers;
        self.scans = snap.scans;
        self.graphics = snap.graphics;
        self.primary = snap.primary;
        self.primary_cleared = snap.primary_cleared;
    }

    /// Whether the story window's output currently ends at the game's read
    /// prompt — the last thing an Inform parser prints before reading a command.
    ///
    /// `pub(crate)` for `glulx_session`'s silent `look`, which must not type a
    /// command at a page that is asking the player a question (SQ-1293).
    pub(crate) fn ends_at_read_prompt(&mut self) -> bool {
        self.primary_scan().is_some_and(|s| s.ends_at_read_prompt())
    }

    /// Return and clear the last `Subheader` room heading the STORY window
    /// captured since the previous call, applying the banner test below to it.
    /// Drained once per turn, alongside `take_transcript`.
    pub fn take_room_heading(&mut self, awaiting_line_input: bool) -> Option<String> {
        let at_command_prompt = awaiting_line_input && self.ends_at_read_prompt();
        self.primary_scan()?.take_room_heading(at_command_prompt)
    }
}

impl StoryScan {
    /// Feed one output run from this window into the room-heading detector.
    ///
    /// The Inform 7 room heading is a `Subheader` run printed on its OWN line, so
    /// a heading is a `Subheader` run that begins at line start (tracked by
    /// `at_line_start`). A `Subheader` run that begins mid-line is an inline
    /// hyperlink — e.g. Superluminal Vagrant Twin renders its "credits"/"land"
    /// command hints as mid-line `Subheader` — and is ignored. A heading run ends
    /// at the next newline or when the style leaves `Subheader`; the LAST heading
    /// in a turn wins, so a banner title printed earlier on its own line is
    /// overwritten by the real room heading.
    ///
    /// A finished heading line is only a *candidate*: an Inform 7 room heading is
    /// joined to the room description that follows it, whereas a title, act
    /// header or content warning is set off by a blank line. See
    /// [`Self::advance_heading_tail`] — that is the test THE BAT needs, whose
    /// title page and prologue each print two own-line `Subheader` banners before
    /// play begins (SQ-0732).
    ///
    /// And the candidate must own its LINE, not merely open it: see
    /// [`Self::line_rest_disqualifies`], the test a game that bolds its object
    /// names needs (SQ-1285).
    fn capture_heading(&mut self, style: GlkStyle, s: &str) {
        let is_sub = style == GlkStyle::Subheader;
        for ch in s.chars() {
            if ch == '\n' {
                if self.in_heading {
                    self.finalize_heading();
                    self.in_heading = false;
                }
                self.advance_heading_tail('\n');
                self.at_line_start = true;
                continue;
            }
            if is_sub {
                if self.at_line_start && !self.in_heading {
                    // A `Subheader` run opening the line DIRECTLY BELOW a candidate
                    // that is one character from confirmation would otherwise displace
                    // it unsettled: `finalize_heading` overwrites `heading_pending`,
                    // and SQ-1285's rule then rejects the newcomer and takes the real
                    // heading with it. Counterfeit Monkey with HIGHLIGHT on prints
                    // exactly that — "**Brown's Lab**" and then "**Professor Brown**,
                    // the Reification of Abstracts researcher, is …" (SQ-1295).
                    //
                    // The verdict cannot be reached HERE, because it depends on what
                    // the displacing line turns out to be: prose opened by a bolded
                    // noun (so the candidate above it was a heading joined to its
                    // description) or a line the newcomer owns outright (so the two
                    // are stacked banners, THE BAT's title page — SQ-0732). So the
                    // candidate is parked and settled once the newcomer's own line
                    // has been judged; see `reject_heading` and `advance_heading_tail`.
                    if self.heading_tail == HeadingTail::LineEnd {
                        self.heading_displaced = self.heading_pending.take();
                        self.heading_tail = HeadingTail::Idle;
                        self.heading_tail_text.clear();
                        self.heading_tail_prose = false;
                        self.heading_line_rest.clear();
                    }
                    self.in_heading = true; // a heading begins only at line start
                }
                if self.in_heading {
                    self.heading_acc.push(ch);
                } else {
                    // A `Subheader` run that began mid-line is an inline link.
                    self.advance_heading_tail(ch);
                }
            } else {
                if self.in_heading {
                    // Non-`Subheader` text on the heading's line ends the heading
                    // run — but not the heading's LINE (Inform prints the
                    // "(on the chair)" parenthetical in roman after the name).
                    self.finalize_heading();
                    self.in_heading = false;
                }
                self.advance_heading_tail(ch);
            }
            self.at_line_start = false;
        }
    }

    /// Feed one non-heading character to the "what follows the candidate?" test.
    ///
    /// The Inform 7 room-description layout puts the heading and the description
    /// in one paragraph — the description starts on the very next line. A banner,
    /// act header, title or content warning is its own paragraph, separated from
    /// the next by a blank line. So text on the line directly below a candidate
    /// confirms it outright; a blank line only makes it *detachable*, and
    /// [`Self::take_room_heading`] decides from there.
    fn advance_heading_tail(&mut self, ch: char) {
        match self.heading_tail {
            HeadingTail::Idle => {}
            HeadingTail::Line => {
                if ch == '\n' {
                    if self.line_rest_disqualifies() {
                        self.reject_heading();
                    } else {
                        // This candidate owns its own line, so anything it displaced
                        // was a banner stacked above a banner rather than a heading
                        // above its description (SQ-1295).
                        self.heading_displaced = None;
                        self.heading_tail = HeadingTail::LineEnd;
                    }
                } else if self.heading_line_rest.chars().count() < HEADING_LINE_REST_CAP {
                    self.heading_line_rest.push(ch);
                }
            }
            HeadingTail::LineEnd => {
                if ch == '\n' {
                    self.heading_tail = HeadingTail::Detached;
                } else {
                    self.confirm_heading();
                }
            }
            HeadingTail::Detached => {
                if self.heading_tail_text.chars().count() < HEADING_TAIL_CAP {
                    self.heading_tail_text.push(ch);
                } else {
                    self.heading_tail_prose = true;
                }
            }
        }
    }

    /// Keep the last [`PROMPT_TAIL_CAP`] chars of this window's output.
    fn note_prompt_tail(&mut self, s: &str) {
        self.prompt_tail.push_str(s);
        let n = self.prompt_tail.chars().count();
        if n > PROMPT_TAIL_CAP {
            let cut = self
                .prompt_tail
                .char_indices()
                .nth(n - PROMPT_TAIL_CAP)
                .map_or(self.prompt_tail.len(), |(i, _)| i);
            self.prompt_tail.drain(..cut);
        }
    }

    /// Whether the primary window's output currently ends at the game's read
    /// prompt — the last thing an Inform parser prints before reading a command.
    fn ends_at_read_prompt(&self) -> bool {
        crate::session::ends_with_read_prompt(&self.prompt_tail)
    }

    /// Whether the roman text that followed the bold run on the candidate's OWN
    /// line rules it out as a room heading.
    ///
    /// An Inform room heading owns its line: the name in bold, and at most the
    /// library's roman parenthetical after it ("Kitchen (on the chair)"). A bold
    /// run that opens a SENTENCE does not — and once a game bolds the names of
    /// its objects, sentences that open with one are everywhere. Counterfeit
    /// Monkey's HIGHLIGHT (`boldening`, an accessibility option the game
    /// advertises) prints every object name in bold type, which Glk carries as
    /// `Subheader` — the very style the heading is printed in. Its `get all`
    /// listing then reads
    ///
    /// ```text
    /// ale: We acquire the ale.
    /// ear: We take the ear.
    /// ```
    ///
    /// with `ale` and `ear` bold at line start, followed by the parser's command
    /// prompt — which is exactly the shape [`Self::take_room_heading`] accepts,
    /// so `get all` in the Midway minted a room called "ear" (SQ-1285). The same
    /// bolding opens paragraphs elsewhere ("**The Aquarium Bookstore** is to the
    /// east."), so the rule has to be about the LINE, not about that one listing.
    ///
    /// Anything but a word disqualifies nothing: trailing spaces, a lone full
    /// stop, the library's `(on the chair)`. It is a WORD following the name on
    /// its own line that says "this is a sentence, not a heading".
    fn line_rest_disqualifies(&self) -> bool {
        let rest = self.heading_line_rest.trim();
        !rest.is_empty() && !rest.starts_with('(') && rest.chars().any(char::is_alphanumeric)
    }

    /// Throw the pending candidate away: it was never a room heading. Leaves any
    /// heading already CONFIRMED this turn standing.
    fn reject_heading(&mut self) {
        // The rejected candidate opened a line of PROSE, which is exactly the thing
        // that confirms a heading sitting above it — so a candidate it displaced was
        // a room heading joined to its description after all (SQ-1295).
        if let Some(displaced) = self.heading_displaced.take() {
            self.last_heading = Some(displaced);
        }
        self.heading_pending = None;
        self.heading_tail = HeadingTail::Idle;
        self.heading_tail_text.clear();
        self.heading_tail_prose = false;
        self.heading_line_rest.clear();
    }

    /// Accept the pending candidate as this turn's room heading.
    fn confirm_heading(&mut self) {
        // A confirmed candidate supersedes whatever it displaced, and the parked one
        // is not confirmed in its own right — see `reject_heading` for the case that
        // promotes it (SQ-1295).
        self.heading_displaced = None;
        if let Some(name) = self.heading_pending.take() {
            self.last_heading = Some(name);
        }
        self.heading_tail = HeadingTail::Idle;
        self.heading_tail_text.clear();
        self.heading_tail_prose = false;
        self.heading_line_rest.clear();
    }

    /// Promote the accumulated `Subheader` text (if any, trimmed non-empty) to
    /// the pending-candidate slot, to be confirmed or rejected by what follows.
    fn finalize_heading(&mut self) {
        let line = self.heading_acc.trim().to_string();
        self.heading_acc.clear();
        if !line.is_empty() {
            self.heading_pending = Some(line);
            self.heading_tail = HeadingTail::Line;
            self.heading_tail_text.clear();
            self.heading_tail_prose = false;
            self.heading_line_rest.clear();
        }
    }

    /// Return and clear the last `Subheader` room heading captured since the
    /// previous call. Drained once per turn, alongside `take_transcript`.
    ///
    /// `awaiting_line_input` says whether the turn ended with the game reading a
    /// line rather than a keypress. Together with the read prompt this backend
    /// saw last, it forms the second half of the banner test: a heading that a
    /// blank line has already detached from the prose below it is a room only if
    /// the player is being handed the parser's **command prompt**.
    ///
    /// Line input alone is not that prompt, and the difference is the whole of
    /// SQ-0733. A page that says "press any key to continue" is obviously not a
    /// turn — THE BAT's title, its act list and its prologue's newspaper
    /// strapline are all own-line `Subheader` lines on such pages, and each used
    /// to mint a room before play began (SQ-0732). But a front-matter page can
    /// just as well read a *line*: cragne Manor's CONTENT WARNING and CONCEPT
    /// WARNING pages each end "Would you still like to continue? (Please type yes
    /// or no.)" and read the answer directly. What no such page does is print the
    /// prompt: only a completed turn hands back "\n>", because only the parser
    /// prints it. So the test is the prompt, not the input kind.
    ///
    /// The halves are deliberately an AND. Either alone loses real rooms:
    /// Adventure in `superbrief` prints "Inside Building", a blank line and then
    /// only its object list, while a room heading can perfectly well be followed
    /// by a cutscene that ends on a keypress.
    fn take_room_heading(&mut self, at_command_prompt: bool) -> Option<String> {
        if self.in_heading {
            self.finalize_heading(); // flush a heading with no trailing separator yet
            self.in_heading = false;
        }
        // A candidate whose own line never ended still has to answer for what
        // shares that line with it — the turn simply stopped before the newline
        // arrived (SQ-1285).
        if self.heading_tail == HeadingTail::Line && self.line_rest_disqualifies() {
            self.reject_heading();
        }
        // The read prompt the game printed on its way to asking for input is not
        // prose: in `superbrief` a room is the heading, a blank line and ">".
        let detached = self.heading_tail == HeadingTail::Detached
            && (self.heading_tail_prose
                || !crate::session::strip_read_prompt(&self.heading_tail_text).trim().is_empty());
        if detached && !at_command_prompt {
            self.heading_pending = None;
            self.heading_tail = HeadingTail::Idle;
            self.heading_tail_text.clear();
            self.heading_tail_prose = false;
        } else {
            self.confirm_heading();
        }
        // A parked candidate is per-turn evidence; it must never be promoted by a
        // rejection that happens on some later turn (SQ-1295).
        self.heading_displaced = None;
        self.last_heading.take()
    }

    /// Reset what a `glk_window_clear` on this window invalidates: the cursor is
    /// back at line start, any partial heading run was wiped with the text, and
    /// the read prompt that stood there is gone. A heading already CONFIRMED
    /// stands — the wiped window carried away the blank line that would have
    /// judged a pending candidate, so settle it now.
    fn on_clear(&mut self) {
        self.at_line_start = true;
        self.in_heading = false;
        self.heading_acc.clear();
        self.prompt_tail.clear();
        if self.heading_tail == HeadingTail::Line && self.line_rest_disqualifies() {
            self.reject_heading(); // a sentence, not a heading — settle it that way
        } else {
            self.confirm_heading();
        }
    }
}

impl AppGlk {
    /// Project the recorded Glk state onto the neutral [`ScreenModel`] by walking
    /// gvm's live window tree (`layout_tree`). Content is looked up by window id
    /// (unchanged); the tree's position-ordered children and border hints carry
    /// the layout directly, with no rect reconstruction.
    pub fn screen_model(&self) -> ScreenModel {
        let (root, content_size) = match &self.layout_tree {
            None => (WinNode::Blank, (0u16, 0u16)),
            Some(tree) => {
                // Root rect = gvm's laid-out screen (incl. any border gutters).
                // Since SQ-1220 that is the whole pane — a proportional split
                // keeps its own undividable cell rather than the screen shrinking
                // to avoid one — so the composite clamp below (SQ-0303) no longer
                // has a margin to withhold, and is kept as the bound it always was.
                let r = tree.rect();
                let size = (r.width.min(u16::MAX as u32) as u16, r.height.min(u16::MAX as u32) as u16);
                (self.convert_tree(tree), size)
            }
        };
        // The page colour is the PRIMARY buffer window's own colour (the app
        // paints the story pane / live input line with model.bg/fg); each other
        // window carries its own colour on its node. `None` → theme default.
        fn primary_colour(node: &WinNode) -> Option<(Option<u32>, Option<u32>)> {
            match node {
                WinNode::Buffer(b) if b.primary => Some((b.bg, b.fg)),
                WinNode::Pair { first, second, .. } => primary_colour(first).or_else(|| primary_colour(second)),
                _ => None,
            }
        }
        let (pbg, pfg) = primary_colour(&root).unwrap_or((None, None));
        let pack = |c: Option<u32>| match c {
            Some(rgb) => crate::state::pack_zcolour(zvm::screen::ZColour::True24(rgb)),
            None => crate::state::pack_zcolour(zvm::screen::ZColour::Default),
        };
        ScreenModel {
            root,
            status: StatusModel::HostManaged,
            bg: pack(pbg),
            fg: pack(pfg),
            content_size,
        }
    }

    /// Recursively convert a gvm [`WinTree`] node into the neutral [`WinNode`].
    /// Zero-area leaves are kept (they are part of the tree); the renderer skips
    /// zero-area areas in T4.
    fn convert_tree(&self, tree: &WinTree) -> WinNode {
        match tree {
            WinTree::Leaf { id, wintype, rect, bg, fg, reverse } => match wintype {
                // Glulx grids are frameless on the generic path; the pair
                // separator carries the border (T4), so pass `None` here.
                WinType::TextGrid => {
                    let mut g = self.grid_node(*id, *rect, None);
                    g.bg = *bg;
                    g.fg = *fg;
                    g.reverse = *reverse;
                    WinNode::Grid(g)
                }
                WinType::TextBuffer => {
                    let mut b = self.buffer_node(*id);
                    b.bg = *bg;
                    b.fg = *fg;
                    WinNode::Buffer(b)
                }
                WinType::Graphics => {
                    let c = self.graphics.get(id);
                    WinNode::Graphics(crate::engine::GraphicsWindow {
                        win: *id,
                        canvas: c.map(|c| c.arc()).unwrap_or_else(|| std::sync::Arc::new(image::RgbaImage::new(1, 1))),
                        version: c.map(|c| c.version).unwrap_or(0),
                        upscale: false,
                    })
                }
                WinType::Pair => unreachable!("pair windows are never tree leaves"),
            },
            WinTree::Pair { vertical, border, split, key_bg, key_fg, first, second, .. } => WinNode::Pair {
                vertical: *vertical,
                split: Split { fixed: *split as u16 },
                border: *border,
                key_bg: *key_bg,
                key_fg: *key_fg,
                first: Box::new(self.convert_tree(first)),
                second: Box::new(self.convert_tree(second)),
            },
        }
    }

    fn grid_node(&self, id: u32, rect: GlkRect, border: Option<bool>) -> GridWindow {
        let g = self.grids.get(&id);
        let cols = g.map(|g| g.width).unwrap_or(rect.width).max(rect.width) as u16;
        let rows = g.map(|g| g.height).unwrap_or(rect.height).max(rect.height) as u16;
        let mut cells = vec![GridCell::default(); cols as usize * rows as usize];
        if let Some(g) = g {
            for (&(r, c), &(ch, bits, fg, bg, link, gs)) in &g.cells {
                if r < rows as u32 && c < cols as u32 {
                    cells[r as usize * cols as usize + c as usize] = GridCell { ch, style: bits, fg, bg, link, glk_style: gs };
                }
            }
        }
        GridWindow {
            win: id,
            fill: None, // v6-only erase fill (SQ-0584)
            cols,
            rows,
            cells,
            active_rows: rows,
            cursor: (1, 1),
            cursor_active: false,
            // Map the Glk border hint to the neutral preference (SQ-0286).
            border: match border {
                None => BorderPref::Unspecified,
                Some(true) => BorderPref::Border,
                Some(false) => BorderPref::NoBorder,
            },
            bg: None,
            fg: None,
            reverse: false,
            px_texts: Vec::new(),
        }
    }

    fn buffer_node(&self, id: u32) -> BufferWindow {
        if self.primary == Some(id) {
            // The primary buffer is mirrored by the app transcript; carry no
            // inline content (the renderer draws it via the transcript path).
            return BufferWindow { win: id, primary: true, ..Default::default() };
        }
        let buf = self.buffers.get(&id);
        let (lines, runs, para, images) = buf.map(|b| log_to_lines(&b.log)).unwrap_or_default();
        let scroll = buf.map(|b| b.scroll).unwrap_or(0);
        BufferWindow { win: id, lines, runs, para, images, scroll, primary: false, bg: None, fg: None, panel: false, px_runs: Vec::new(), reads_input: false }
    }
}

// ── log → lines helper ─────────────────────────────────────────────────────────

/// The parallel per-line vecs `log_to_lines` produces: `(lines, per-line runs,
/// per-line paragraph format, per-line inline image)`, all kept the same length.
type LogLines = (Vec<String>, Vec<Vec<StyleRun>>, Vec<crate::state::ParaFmt>, Vec<Option<crate::inline_image::InlineImage>>);

/// Split a buffer window's styled log into `(lines, per-line runs, per-line
/// image)`, merging adjacent same-style chars into one [`StyleRun`]. The three
/// vecs are always kept the same length: an image occupies its own logical
/// line (a fresh line is started before it if the current line has content,
/// and a fresh line is always started after it).
fn log_to_lines(
    log: &[BufElem],
) -> LogLines {
    use crate::state::ParaFmt;
    let mut lines: Vec<String> = vec![String::new()];
    let mut runs: Vec<Vec<StyleRun>> = vec![Vec::new()];
    // Per-line layout; `None` until the first content char sets it from its run.
    let mut para: Vec<Option<ParaFmt>> = vec![None];
    let mut images: Vec<Option<crate::inline_image::InlineImage>> = vec![None];
    for elem in log {
        match elem {
            BufElem::Text { bits, fg, bg, link, para: pf, glk_style, text } => {
                for ch in text.chars() {
                    if ch == '\n' {
                        lines.push(String::new());
                        runs.push(Vec::new());
                        para.push(None);
                        images.push(None);
                        continue;
                    }
                    let li = lines.len() - 1;
                    let col = lines[li].chars().count();
                    if para[li].is_none() {
                        para[li] = Some(*pf);
                    }
                    lines[li].push(ch);
                    // A run is emitted whenever any styling is active (bits, a
                    // colour, a hyperlink, or a non-Normal Glk style whose theme
                    // colour slot must apply at render, SQ-0331).
                    if *bits != 0 || *fg != 0 || *bg != 0 || *link != 0 || *glk_style != 0 {
                        let r = &mut runs[li];
                        match r.last_mut() {
                            Some(last)
                                if last.bits == *bits
                                    && last.fg == *fg
                                    && last.bg == *bg
                                    && last.link == *link
                                    && last.glk_style == *glk_style
                                    && last.end == col =>
                            {
                                last.end = col + 1
                            }
                            _ => r.push(StyleRun { start: col, end: col + 1, bits: *bits, fg: *fg, bg: *bg, link: *link, glk_style: *glk_style }),
                        }
                    }
                }
            }
            BufElem::Image(img) => {
                // An image occupies its own logical line: start a fresh line
                // before it if the current line already has content.
                if !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                    lines.push(String::new());
                    runs.push(Vec::new());
                    para.push(None);
                    images.push(None);
                }
                if let Some(last) = images.last_mut() {
                    *last = Some(img.clone());
                }
                // Always start a fresh line after the image.
                lines.push(String::new());
                runs.push(Vec::new());
                para.push(None);
                images.push(None);
            }
        }
    }
    let para = para.into_iter().map(|p| p.unwrap_or_default()).collect();
    (lines, runs, para, images)
}

// ── GlkBackend impl ────────────────────────────────────────────────────────────

impl GlkBackend for AppGlk {
    fn screen_size(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    fn window_open(&mut self, id: u32, wintype: WinType) {
        match wintype {
            WinType::TextGrid => {
                self.grids.entry(id).or_default();
            }
            WinType::TextBuffer => {
                self.buffers.entry(id).or_default();
                if self.primary.is_none() {
                    self.primary = Some(id);
                }
            }
            WinType::Pair => {}
            WinType::Graphics => {}
        }
    }

    fn window_close(&mut self, id: u32) {
        self.grids.remove(&id);
        self.buffers.remove(&id);
        self.scans.remove(&id);
        self.graphics.remove(&id);
        self.layout.retain(|&(wid, _, _, _)| wid != id);
        if self.primary == Some(id) {
            self.primary = None;
        }
    }

    fn window_layout(&mut self, wins: &[(u32, WinType, GlkRect, Option<bool>)]) {
        self.layout = wins.to_vec();
        for &(id, ty, rect, _border) in wins {
            if ty == WinType::TextGrid {
                let g = self.grids.entry(id).or_default();
                g.width = rect.width;
                g.height = rect.height;
            }
        }
        for &(id, ty, rect, _border) in wins {
            if ty == WinType::Graphics {
                let (cw, ch) = (rect.width * self.char_px.0, rect.height * self.char_px.1);
                if let Some(c) = self.graphics.get_mut(&id) {
                    c.resize(cw, ch);
                }
            }
        }
    }

    fn window_tree(&mut self, tree: Option<WinTree>) {
        self.layout_tree = tree;
    }

    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        self.put_text_attr(win, style, StyleColour::default(), StyleAttrs::default(), 0, s);
    }

    fn put_text_attr(&mut self, win: u32, style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, link: u32, s: &str) {
        // Every buffer is scanned, not just today's primary: the window a game
        // prints its story into can become the primary only at the END of the
        // turn it opened in (SQ-1241 — see `AppGlk::scans`).
        let scan = self.scan(win);
        scan.capture_heading(style, s);
        scan.note_prompt_tail(s);
        let (bits, fg, bg) = resolve_glk_colour(style, colour, attrs);
        let para = resolve_glk_para(attrs);
        let buf = self.buffers.entry(win).or_default();
        buf.log.push(BufElem::Text { bits, fg, bg, link, para, glk_style: style.to_num() as u8, text: s.to_owned() });
    }

    fn grid_put(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, s: &str) {
        self.grid_put_attr(win, x, y, style, StyleColour::default(), StyleAttrs::default(), 0, s);
    }

    fn grid_put_attr(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, link: u32, s: &str) {
        let (bits, fg, bg) = resolve_glk_colour(style, colour, attrs);
        let gs = style.to_num() as u8;
        let g = self.grids.entry(win).or_default();
        for (i, ch) in s.chars().enumerate() {
            g.cells.insert((y, x + i as u32), (ch, bits, fg, bg, link, gs));
        }
    }

    fn grid_clear(&mut self, win: u32) {
        if let Some(g) = self.grids.get_mut(&win) {
            g.cells.clear();
        }
    }

    fn window_clear(&mut self, win: u32) {
        if let Some(b) = self.buffers.get_mut(&win) {
            b.log.clear();
            b.drained = 0;
        }
        if let Some(c) = self.graphics.get_mut(&win) {
            let (w, h) = (c.img.width(), c.img.height());
            c.erase_rect(0, 0, w, h);
        }
        // A cleared window puts the cursor back at line start, so a heading
        // printed at the top of the fresh window is a valid line-start heading.
        // Reset that window's own scan (SQ-1241) — a window that is not primary
        // today may be the story window tomorrow.
        if let Some(scan) = self.scans.get_mut(&win) {
            scan.on_clear();
        }
        if Some(win) == self.primary {
            // Signal the app to pin the upcoming reprint to a fresh screen
            // instead of appending a fresh copy (menu redraws). (SQ-0403)
            self.primary_cleared = true;
        }
    }

    fn flush(&mut self) {}

    /// The system timezone's UTC offset (seconds east of Greenwich) at the given
    /// instant, for the Glk `_local` date/time selectors. `jiff` resolves the
    /// offset per instant, so DST is correct at any queried time (not just now),
    /// and it is thread-safe (no `localtime_r`/`setenv` race) and cross-platform
    /// (system tzdb on Unix/macOS, a bundled tzdb on Windows). Out-of-range
    /// instants → `None` → the selectors fall back to UTC.
    fn local_utc_offset_seconds(&self, epoch_seconds: i64) -> Option<i32> {
        let ts = jiff::Timestamp::from_second(epoch_seconds).ok()?;
        Some(jiff::tz::TimeZone::system().to_offset(ts).seconds())
    }

    fn char_pixels(&self) -> (u32, u32) {
        self.char_px
    }

    fn image_info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.picts.info(resnum)
    }

    fn data_resource(&mut self, num: u32) -> Option<(Vec<u8>, bool)> {
        self.picts.data_resource(num)
    }

    fn default_style_colours(&self, wintype: WinType, style: u32) -> Option<(Option<u32>, Option<u32>)> {
        // SQ-0803: the theme paints a colour per Glk STYLE CLASS (the SQ-0331
        // slots, which a discovered garglk.ini populates), falling back to the
        // pane's base for a style it leaves alone — so report the slot the
        // renderer would actually use, not just the pane base. A style class
        // outside 0..style_NUMSTYLES has no rendered colour to report.
        let row = match wintype {
            WinType::TextBuffer => 0,
            WinType::TextGrid => 1,
            WinType::Pair | WinType::Graphics => return None,
        };
        self.theme_styles[row].get(style as usize).copied()
    }

    fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .fill_rect(color, left, top, w, h);
    }

    fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .erase_rect(left, top, w, h);
    }

    fn graphics_set_background(&mut self, win: u32, color: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .set_background(color);
    }

    fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) -> bool {
        // Buffer-window target: `x` is really the Glk imagealign flag; the image
        // flows inline with the window's text rather than onto a pixel canvas.
        if self.buffers.contains_key(&win) {
            let Some(src) = self.picts.image(resnum) else { return false };
            let img = crate::inline_image::InlineImage {
                pixels: std::sync::Arc::new(src.to_rgba8()),
                align: crate::inline_image::ImageAlign::from_glk(x as u32),
                scaled: scale, margin_px: None,
            };
            if let Some(buf) = self.buffers.get_mut(&win) {
                buf.log.push(BufElem::Image(img));
            }
            return true;
        }
        // Graphics-window target: existing canvas path.
        let Some(src) = self.picts.image(resnum) else { return false };
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .draw_image(&src, x, y, scale);
        true
    }

    fn schannel_create(&mut self, rock: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannels.insert(id, SoundChannel { rock, volume: 0x10000, paused: false });
        id
    }
    fn schannel_destroy(&mut self, chan: u32) {
        self.schannels.remove(&chan);
        self.sound_ops.push(crate::session::SchannelOp::Destroy { chan });
    }
    fn schannel_iterate(&mut self, chan: u32) -> (u32, u32) {
        let next = if chan == 0 {
            self.schannels.keys().next().copied()
        } else {
            self.schannels.range((chan + 1)..).next().map(|(k, _)| *k)
        };
        match next {
            Some(id) => (id, self.schannels.get(&id).map(|c| c.rock).unwrap_or(0)),
            None => (0, 0),
        }
    }
    fn schannel_get_rock(&mut self, chan: u32) -> u32 {
        self.schannels.get(&chan).map(|c| c.rock).unwrap_or(0)
    }
    fn schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32 {
        let (volume, paused) = self
            .schannels
            .get(&chan)
            .map(|c| (c.volume, c.paused))
            .unwrap_or((0x10000, false));
        self.sound_ops.push(crate::session::SchannelOp::Play { chan, snd, repeats, notify, volume, paused });
        1
    }
    fn schannel_stop(&mut self, chan: u32) {
        self.sound_ops.push(crate::session::SchannelOp::Stop { chan });
    }
    fn schannel_set_volume(&mut self, chan: u32, vol: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.volume = vol;
        }
        self.sound_ops.push(crate::session::SchannelOp::SetVolume { chan, vol });
    }
    fn schannel_create_ext(&mut self, rock: u32, volume: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannels.insert(id, SoundChannel { rock, volume, paused: false });
        id
    }
    fn schannel_pause(&mut self, chan: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.paused = true;
        }
        self.sound_ops.push(crate::session::SchannelOp::Pause { chan });
    }
    fn schannel_unpause(&mut self, chan: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.paused = false;
        }
        self.sound_ops.push(crate::session::SchannelOp::Unpause { chan });
    }
    fn schannel_set_volume_ext(&mut self, chan: u32, vol: u32, duration_ms: u32, notify: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.volume = vol;
        }
        self.sound_ops.push(crate::session::SchannelOp::SetVolumeExt { chan, vol, duration_ms, notify });
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;

    fn rect(left: u32, top: u32, width: u32, height: u32) -> GlkRect {
        GlkRect { left, top, width, height }
    }

    /// Test helper: a `WinTree` leaf (no per-window colour).
    fn leaf(id: u32, wintype: WinType, r: GlkRect) -> WinTree {
        WinTree::Leaf { id, wintype, rect: r, bg: None, fg: None, reverse: false }
    }

    /// Test helper: a `WinTree` leaf carrying a packed bg/fg colour.
    fn leaf_col(id: u32, wintype: WinType, r: GlkRect, bg: Option<u32>, fg: Option<u32>) -> WinTree {
        WinTree::Leaf { id, wintype, rect: r, bg, fg, reverse: false }
    }

    /// Bounding rect of two child rects (a pair node's own rect).
    fn union(a: GlkRect, b: GlkRect) -> GlkRect {
        let l = a.left.min(b.left);
        let t = a.top.min(b.top);
        let r = (a.left + a.width).max(b.left + b.width);
        let bo = (a.top + a.height).max(b.top + b.height);
        GlkRect { left: l, top: t, width: r - l, height: bo - t }
    }

    /// Test helper: an Above/Below (vertical) pair; `split` = the first (top)
    /// child's row count.
    fn vpair(split: u32, first: WinTree, second: WinTree) -> WinTree {
        let rect = union(first.rect(), second.rect());
        WinTree::Pair { vertical: true, border: false, split, rect, key_bg: None, key_fg: None, first: Box::new(first), second: Box::new(second) }
    }

    /// Test helper: a Left/Right (horizontal) pair; `split` = the first (left)
    /// child's column count.
    fn hpair(split: u32, first: WinTree, second: WinTree) -> WinTree {
        let rect = union(first.rect(), second.rect());
        WinTree::Pair { vertical: false, border: false, split, rect, key_bg: None, key_fg: None, first: Box::new(first), second: Box::new(second) }
    }

    #[test]
    fn local_offset_is_some_and_plausible() {
        // We can't assert a specific zone on an unknown CI host, but the hook
        // must resolve the system zone to a sane offset at a real instant
        // (SQ-0317 T4); real-TZ correctness is left to the SQ-0312 sweep.
        let glk = AppGlk::new(80, 24);
        let off = glk
            .local_utc_offset_seconds(1_784_030_400) // 2026-07-14 12:00:00 UTC
            .expect("system timezone resolves to some offset");
        assert!(off.abs() <= 14 * 3600, "offset {off}s within ±14h");
        // An absurd instant (out of jiff's timestamp range) falls back to None.
        assert_eq!(glk.local_utc_offset_seconds(i64::MAX), None);
    }

    #[test]
    fn glk_styles_map_to_bits() {
        assert_eq!(glk_style_bits(GlkStyle::Normal), 0);
        assert_eq!(glk_style_bits(GlkStyle::Emphasized), 0x04); // italic
        assert_eq!(glk_style_bits(GlkStyle::Header), 0x02);
        assert_eq!(glk_style_bits(GlkStyle::Alert), 0x03);
        assert_eq!(glk_style_bits(GlkStyle::Preformatted), 0x08);
    }

    #[test]
    fn default_style_colours_reports_theme_pairs_per_wintype() {
        // SQ-0315: AppGlk reports the pushed theme pairs through the
        // GlkBackend hook — buffer pair for TextBuffer, grid pair for TextGrid,
        // nothing for Pair/Graphics. Unset → (None, None) pairs (still Some
        // outer: the host knows its answer is "terminal default").
        let mut glk = AppGlk::new(80, 24);
        assert_eq!(glk.default_style_colours(WinType::TextBuffer, 0), Some((None, None)));
        let mut pairs: GlkStylePairs = [[(None, None); 11]; 2];
        pairs[0] = [(Some(0x00C5C8C6), Some(0x001D1F21)); 11];
        pairs[1] = [(None, Some(0x00303030)); 11];
        glk.set_theme_colours(pairs);
        assert_eq!(
            glk.default_style_colours(WinType::TextBuffer, 0),
            Some((Some(0x00C5C8C6), Some(0x001D1F21)))
        );
        // A theme with no per-style colours reports the same pane pair for every
        // style class.
        assert_eq!(
            glk.default_style_colours(WinType::TextBuffer, 5),
            Some((Some(0x00C5C8C6), Some(0x001D1F21)))
        );
        assert_eq!(
            glk.default_style_colours(WinType::TextGrid, 0),
            Some((None, Some(0x00303030))),
            "grid fg is terminal-default -> None channel"
        );
        assert_eq!(glk.default_style_colours(WinType::Graphics, 0), None);
        assert_eq!(glk.default_style_colours(WinType::Pair, 0), None);
    }

    #[test]
    fn default_style_colours_reports_the_per_style_slot() {
        // SQ-0803: a theme that paints one Glk style class differently (here
        // style_User2 = 10, glk.h) must be reported for THAT style, with every
        // other style still answering with the pane base. This is the half of
        // SQ-0319 that Kerkerkruip's 0xF400A1 sentinel probes.
        let mut glk = AppGlk::new(80, 24);
        let mut pairs: GlkStylePairs = [[(Some(0x00C5C8C6), Some(0x001D1F21)); 11]; 2];
        pairs[0][10] = (Some(0x00F400A1), Some(0x00FFFFFF));
        glk.set_theme_colours(pairs);
        assert_eq!(
            glk.default_style_colours(WinType::TextBuffer, 10),
            Some((Some(0x00F400A1), Some(0x00FFFFFF))),
            "style_User2 reports its own slot, not the pane base"
        );
        assert_eq!(
            glk.default_style_colours(WinType::TextBuffer, 0),
            Some((Some(0x00C5C8C6), Some(0x001D1F21))),
            "an unslotted style still reports the pane base"
        );
        assert_eq!(
            glk.default_style_colours(WinType::TextGrid, 10),
            Some((Some(0x00C5C8C6), Some(0x001D1F21))),
            "the buffer slot does not leak into the grid row"
        );
        // glk.h: style_NUMSTYLES = 11, so 11 is out of range and has no colour.
        assert_eq!(glk.default_style_colours(WinType::TextBuffer, 11), None);
    }

    #[test]
    fn theme_style_colours_converts_only_concrete_rgb() {
        // Rgb channels convert to 0x00RRGGBB; a named ANSI colour or an unset
        // channel is terminal-defined, so it honestly reports None. `transcript`
        // still rides the legacy field (SQ-0309: kept, see module docs above);
        // `status_bar` now reads through the theme, which has no setter here, so
        // the grid-half assertion instead relies on the terminal default's
        // `status_bar` never carrying a concrete Rgb channel.
        use ratatui::style::{Color, Style};
        let mut cs = crate::colors::ColorScheme::default();
        cs.transcript = Style::new().fg(Color::Rgb(0xC5, 0xC8, 0xC6)).bg(Color::Rgb(0x1D, 0x1F, 0x21));
        let sb = cs.theme.get("status_bar").style;
        assert!(!matches!(sb.fg, Some(Color::Rgb(..))), "precondition: status_bar fg isn't concrete Rgb");
        assert!(!matches!(sb.bg, Some(Color::Rgb(..))), "precondition: status_bar bg isn't concrete Rgb");
        let pairs = theme_style_colours(&cs);
        assert_eq!(pairs[0][0], (Some(0x00C5C8C6), Some(0x001D1F21)));
        assert_eq!(pairs[1][0], (None, None));
    }

    #[test]
    fn theme_style_colours_resolves_each_glk_style_slot_over_the_base() {
        // SQ-0803: a per-Glk-style slot (what a garglk.ini `tcolor N` / `gcolor N`
        // populates) is what the renderer paints for that style, so it is what we
        // report; a style with no slot inherits the element base, exactly as
        // `render::resolve_glk_channel` resolves it with no game-set colour. A
        // named ANSI slot colour IS painted but has no knowable RGB, so it stays
        // honestly unknown rather than falling back to the base.
        use ratatui::style::{Color, Style};
        let mut cs = crate::colors::ColorScheme::default();
        cs.transcript = Style::new().fg(Color::Rgb(0xC5, 0xC8, 0xC6)).bg(Color::Rgb(0x1D, 0x1F, 0x21));
        cs.glk_styles[0][10] = Style::new().fg(Color::Rgb(0xF4, 0x00, 0xA1)).bg(Color::Rgb(0xFF, 0xFF, 0xFF));
        cs.glk_styles[0][9] = Style::new().fg(Color::Cyan);
        cs.glk_styles[1][3] = Style::new().fg(Color::Rgb(0x01, 0x02, 0x03));
        let pairs = theme_style_colours(&cs);
        assert_eq!(pairs[0][10], (Some(0x00F400A1), Some(0x00FFFFFF)), "User2 slot reported");
        assert_eq!(
            pairs[0][9],
            (None, Some(0x001D1F21)),
            "a named ANSI slot fg is unknowable; the unslotted bg inherits the base"
        );
        assert_eq!(pairs[0][0], (Some(0x00C5C8C6), Some(0x001D1F21)), "Normal is the base");
        assert_eq!(pairs[0][5], (Some(0x00C5C8C6), Some(0x001D1F21)), "unslotted style is the base");
        assert_eq!(pairs[1][3], (Some(0x00010203), None), "the grid row resolves independently");
    }

    #[test]
    fn non_primary_buffer_carries_inline_image() {
        // A resolvable Pict needs a Blorb, which the test harness lacks (see
        // `take_transcript_elems_coalesces_text_and_keeps_image_between`), so
        // seed the non-primary buffer's log directly: Text, Image, Text.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // primary
        glk.window_open(2, WinType::TextBuffer); // non-primary
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let log = &mut glk.buffers.get_mut(&2).unwrap().log;
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, link: 0, para: crate::state::ParaFmt::default(), glk_style: 0, text: "a\n".into() });
        log.push(BufElem::Image(dummy));
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, link: 0, para: crate::state::ParaFmt::default(), glk_style: 0, text: "b".into() });

        let bw = glk.buffer_node(2);
        assert_eq!(bw.lines.len(), bw.runs.len());
        assert_eq!(bw.lines.len(), bw.images.len());
        let img_idx = bw.images.iter().position(|o| o.is_some()).expect("image line present");
        assert_eq!(bw.lines[img_idx], "", "image occupies its own logical line");
        assert!(img_idx > 0 && img_idx < bw.lines.len() - 1, "image line sits between a and b");
    }

    #[test]
    fn primary_window_clear_signals_erase_and_resets_the_drain() {
        // SQ-0403: an Inform 7 menu clears the primary buffer and reprints on
        // every keypress. The clear must (a) flag a screen clear so the app pins
        // the reprint to a fresh screen, and (b) reset the drain so the reprint
        // is drained ONCE, not stacked on the previous copy.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // first buffer -> primary
        assert!(!glk.take_primary_cleared(), "no clear yet");

        glk.put_text(1, GlkStyle::Normal, "MENU v1\n");
        let (first, _) = glk.take_transcript();
        assert!(first.contains("MENU v1"));

        // Menu redraw: clear the primary window, then reprint.
        glk.window_clear(1);
        assert!(glk.take_primary_cleared(), "clearing the primary buffer flags a screen clear");
        assert!(!glk.take_primary_cleared(), "the flag resets when taken");
        glk.put_text(1, GlkStyle::Normal, "MENU v2\n");
        let (second, _) = glk.take_transcript();
        assert!(second.contains("MENU v2"), "reprint present: {second:?}");
        assert!(!second.contains("MENU v1"), "cleared content is not re-drained: {second:?}");

        // A grid (upper-window) clear must NOT flag it — the grid redraws in place.
        glk.window_open(2, WinType::TextGrid);
        glk.window_clear(2);
        assert!(!glk.take_primary_cleared(), "grid clears don't trigger the anchor");
    }

    #[test]
    fn grid_over_buffer_builds_pair_tree() {
        // A 1-row TextGrid (id 2) stacked above an 80x23 TextBuffer (id 1).
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextGrid);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 1, 80, 23), Some(true)),
            (2, WinType::TextGrid, rect(0, 0, 80, 1), Some(true)),
        ]);
        glk.window_tree(Some(vpair(
            1,
            leaf(2, WinType::TextGrid, rect(0, 0, 80, 1)),
            leaf(1, WinType::TextBuffer, rect(0, 1, 80, 23)),
        )));
        let model = glk.screen_model();
        match &model.root {
            WinNode::Pair { vertical, split, first, second, .. } => {
                assert!(*vertical, "grid-above-buffer is a vertical stack");
                assert_eq!(split.fixed, 1, "the 1-row grid is the fixed first child");
                assert!(matches!(**first, WinNode::Grid(_)), "top child is the grid");
                assert!(matches!(**second, WinNode::Buffer(_)), "bottom child is the buffer");
            }
            other => panic!("expected a Pair, got {other:?}"),
        }
        // The buffer is the primary (mirrored by the transcript).
        assert_eq!(glk.primary(), Some(1));
        assert!(model.grid().is_some(), "the tree exposes a grid node");
    }

    /// SQ-0329: `/dump-windows` formats the live tree with each window's type, id,
    /// size, origin, and per-window colour; the primary buffer is marked.
    #[test]
    fn window_dump_formats_tree_with_colours() {
        let mut glk = AppGlk::new(60, 14);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextGrid);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 1, 60, 13), Some(true)),
            (2, WinType::TextGrid, rect(0, 0, 60, 1), Some(true)),
        ]);
        glk.window_tree(Some(vpair(
            1,
            leaf_col(2, WinType::TextGrid, rect(0, 0, 60, 1), Some(0x00FF_FFFF), Some(0x0000_0000)),
            leaf_col(1, WinType::TextBuffer, rect(0, 1, 60, 13), Some(0x0012_3456), None),
        )));
        let lines = glk.window_dump_lines();
        assert_eq!(lines[0], "Window layout (60x14):");
        assert!(
            lines.iter().any(|l| l.contains("Pair") && l.contains("vertical") && l.contains("split=1")),
            "pair line missing: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Grid id=2")
                && l.contains("60x1") && l.contains("@(0,0)")
                && l.contains("bg=#FFFFFF") && l.contains("fg=#000000")),
            "grid line missing: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Buffer id=1 (primary)") && l.contains("bg=#123456")),
            "buffer line missing: {lines:?}"
        );
    }

    #[test]
    fn three_window_split_nests() {
        // Grid (id 3, top row) over a left/right buffer split (ids 1, 2).
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextBuffer);
        glk.window_open(3, WinType::TextGrid);
        glk.window_layout(&[
            (3, WinType::TextGrid, rect(0, 0, 80, 1), Some(true)),
            (1, WinType::TextBuffer, rect(0, 1, 40, 23), Some(true)),
            (2, WinType::TextBuffer, rect(40, 1, 40, 23), Some(true)),
        ]);
        glk.window_tree(Some(vpair(
            1,
            leaf(3, WinType::TextGrid, rect(0, 0, 80, 1)),
            hpair(
                40,
                leaf(1, WinType::TextBuffer, rect(0, 1, 40, 23)),
                leaf(2, WinType::TextBuffer, rect(40, 1, 40, 23)),
            ),
        )));
        let model = glk.screen_model();
        // Top-level: vertical pair (grid over the rest).
        let WinNode::Pair { vertical, first, second, .. } = &model.root else {
            panic!("expected a top-level Pair");
        };
        assert!(*vertical);
        assert!(matches!(**first, WinNode::Grid(_)));
        // The lower region is a horizontal (side-by-side) pair of two buffers.
        let WinNode::Pair { vertical: v2, first: f2, second: s2, .. } = &**second else {
            panic!("expected a nested Pair for the two buffers");
        };
        assert!(!*v2, "two side-by-side buffers form a horizontal pair");
        assert!(matches!(**f2, WinNode::Buffer(_)));
        assert!(matches!(**s2, WinNode::Buffer(_)));
    }

    #[test]
    fn input_window_becomes_primary_over_first_opened() {
        // narco's pattern: id 1 opens first (the default "primary") but is a
        // near-empty decorative pane; the game does all its story, prompt and
        // line input in id 2. The primary — the window the inline prompt and
        // transcript follow — must track the LINE-INPUT window, not the
        // first-opened one. (SQ-0337)
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // first-opened default primary
        glk.window_open(2, WinType::TextBuffer);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 0, 40, 24), Some(true)),
            (2, WinType::TextBuffer, rect(40, 0, 40, 24), Some(true)),
        ]);
        glk.window_tree(Some(hpair(
            40,
            leaf(1, WinType::TextBuffer, rect(0, 0, 40, 24)),
            leaf(2, WinType::TextBuffer, rect(40, 0, 40, 24)),
        )));
        assert_eq!(glk.primary(), Some(1), "first-opened is the default primary");
        glk.set_input_window(Some(2));
        assert_eq!(glk.primary(), Some(2), "line-input window overrides first-opened");
        let model = glk.screen_model();
        let WinNode::Pair { first, second, .. } = &model.root else { panic!("expected a Pair") };
        let (WinNode::Buffer(b1), WinNode::Buffer(b2)) = (&**first, &**second) else {
            panic!("expected two Buffer leaves");
        };
        assert!(!b1.primary, "first-opened buffer is no longer primary");
        assert!(b2.primary, "the line-input buffer is now primary");
    }

    #[test]
    fn set_input_window_ignores_none_and_non_buffers() {
        // The fallback stays byte-identical for the common cases: a char-input
        // turn (None), input on a non-buffer window, or an unknown id must all
        // leave the first-opened primary untouched. (SQ-0337)
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextGrid);
        assert_eq!(glk.primary(), Some(1));
        glk.set_input_window(None);
        assert_eq!(glk.primary(), Some(1), "None (char-input turn) leaves the default primary");
        glk.set_input_window(Some(2));
        assert_eq!(glk.primary(), Some(1), "input on a grid does not hijack the primary buffer");
        glk.set_input_window(Some(99));
        assert_eq!(glk.primary(), Some(1), "an unknown window id is ignored");
    }

    #[test]
    fn put_text_styles_inline_buffer() {
        // Two buffers: id 1 is primary (drained), id 2 is inline.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextBuffer);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 0, 40, 24), Some(true)),
            (2, WinType::TextBuffer, rect(40, 0, 40, 24), Some(true)),
        ]);
        glk.window_tree(Some(hpair(
            40,
            leaf(1, WinType::TextBuffer, rect(0, 0, 40, 24)),
            leaf(2, WinType::TextBuffer, rect(40, 0, 40, 24)),
        )));
        glk.put_text(2, GlkStyle::Normal, "ab");
        glk.put_text(2, GlkStyle::Header, "CD");
        glk.put_text(2, GlkStyle::Normal, "\nx");

        let model = glk.screen_model();
        // Find the inline (non-primary) buffer node.
        fn find_buffers(n: &WinNode, out: &mut Vec<BufferWindow>) {
            match n {
                WinNode::Buffer(b) => out.push(b.clone()),
                WinNode::Pair { first, second, .. } => {
                    find_buffers(first, out);
                    find_buffers(second, out);
                }
                _ => {}
            }
        }
        let mut bufs = Vec::new();
        find_buffers(&model.root, &mut bufs);
        let inline = bufs.iter().find(|b| !b.primary).expect("an inline buffer exists");
        assert_eq!(inline.lines, vec!["abCD".to_string(), "x".to_string()]);
        // "CD" (cols 2..4) is bold (Header → 0x02), merged into one run; it
        // carries the Header Glk style class (3) for the theme colour slot.
        assert_eq!(inline.runs[0], vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 3 }]);
        assert!(inline.runs[1].is_empty());
    }

    #[test]
    fn primary_text_is_drainable() {
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_layout(&[(1, WinType::TextBuffer, rect(0, 0, 80, 24), Some(true))]);
        glk.window_tree(Some(leaf(1, WinType::TextBuffer, rect(0, 0, 80, 24))));
        glk.put_text(1, GlkStyle::Normal, "You are here. ");
        glk.put_text(1, GlkStyle::Emphasized, "Look!");
        let (text, chunks) = glk.take_transcript();
        assert_eq!(text, "You are here. Look!");
        assert_eq!(chunks, vec![
            (14, 0u8, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0u32, crate::state::ParaFmt::default(), 0, false),
            (5, 0x04u8, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0u32, crate::state::ParaFmt::default(), 1, false), // Emphasized → style class 1
        ]);
        // A second drain returns only new text.
        glk.put_text(1, GlkStyle::Normal, " More.");
        let (text2, _) = glk.take_transcript();
        assert_eq!(text2, " More.");
        // The primary buffer node carries no inline content.
        let model = glk.screen_model();
        if let WinNode::Buffer(b) = &model.root {
            assert!(b.primary && b.lines.is_empty());
        } else {
            panic!("single buffer is the root");
        }
    }

    #[test]
    fn grid_put_and_clear_update_cells() {
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextGrid);
        glk.window_layout(&[(1, WinType::TextGrid, rect(0, 0, 10, 2), Some(true))]);
        glk.window_tree(Some(leaf(1, WinType::TextGrid, rect(0, 0, 10, 2))));
        glk.grid_put(1, 2, 0, GlkStyle::Header, "Hi");
        let model = glk.screen_model();
        let g = model.grid().expect("grid node");
        assert_eq!((g.cols, g.rows), (10, 2));
        // 1-based (row 1, col 3) holds 'H' bold; col 4 holds 'i'.
        assert_eq!(g.cell(1, 3).ch, 'H');
        assert_eq!(g.cell(1, 3).style, 0x02);
        assert_eq!(g.cell(1, 4).ch, 'i');
        // Clear empties the cells.
        glk.grid_clear(1);
        let g2 = glk.screen_model();
        assert_eq!(g2.grid().unwrap().cell(1, 3).ch, ' ');
    }

    #[test]
    fn resolve_glk_colour_packs_24bit_and_reverse() {
        use zvm::screen::ZColour;
        // fg/bg become packed True24; the reverse hint sets bit 0x01.
        let sc = StyleColour { fg: Some(0x00AA_BBCC), bg: Some(0x0011_2233), reverse: true };
        let (bits, fg, bg) = resolve_glk_colour(GlkStyle::Normal, sc, StyleAttrs::default());
        assert_eq!(bits, 0x01);
        assert_eq!(fg, crate::state::pack_zcolour(ZColour::True24(0x00AA_BBCC)));
        assert_eq!(bg, crate::state::pack_zcolour(ZColour::True24(0x0011_2233)));
        // No hints set: only the style-class bits, no colour. (The honor gate is
        // applied at render time, not here.)
        assert_eq!(
            resolve_glk_colour(GlkStyle::Header, StyleColour::default(), StyleAttrs::default()),
            (0x02, 0, 0)
        );
    }

    #[test]
    fn resolve_glk_colour_layers_weight_and_oblique_hints() {
        // SQ-0317: a set hint overrides the class default; an unset hint keeps it.
        let plain = StyleColour::default();
        let bits = |style, attrs| resolve_glk_colour(style, plain, attrs).0;
        // Weight 1 on Normal → bold added to a class with no intrinsic bits.
        assert_eq!(bits(GlkStyle::Normal, StyleAttrs { weight: Some(1), ..Default::default() }), 0x02);
        // Oblique 1 on Normal → italic.
        assert_eq!(bits(GlkStyle::Normal, StyleAttrs { oblique: Some(1), ..Default::default() }), 0x04);
        // Weight 0 on Header strips the class-intrinsic bold.
        assert_eq!(bits(GlkStyle::Header, StyleAttrs { weight: Some(0), ..Default::default() }), 0);
        // "Lighter" (-1) has no terminal rendering → treated as not-bold.
        assert_eq!(
            bits(GlkStyle::Header, StyleAttrs { weight: Some(u32::MAX), ..Default::default() }),
            0
        );
        // No hints → class default preserved (Emphasized keeps its intrinsic look).
        assert_eq!(
            bits(GlkStyle::Emphasized, StyleAttrs::default()),
            glk_style_bits(GlkStyle::Emphasized)
        );
        // Hints layer: oblique adds italic on top of Header's intrinsic bold.
        assert_eq!(bits(GlkStyle::Header, StyleAttrs { oblique: Some(1), ..Default::default() }), 0x06);
    }

    #[test]
    fn buffer_colour_flows_to_transcript() {
        use zvm::screen::ZColour;
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        let red = StyleColour { fg: Some(0x00FF_0000), bg: None, reverse: false };
        glk.put_text_attr(1, GlkStyle::Normal, red, StyleAttrs::default(), 0, "hi");
        let (text, chunks) = glk.take_transcript();
        assert_eq!(text, "hi");
        assert_eq!(chunks.len(), 1);
        let (n, _bits, fg, bg, _link, _para, _gs, _nw) = chunks[0];
        assert_eq!(n, 2);
        assert_eq!(fg, ZColour::True24(0x00FF_0000), "24-bit fg carried losslessly");
        assert_eq!(bg, ZColour::Default);
    }

    #[test]
    fn buffer_paragraph_layout_flows_to_transcript_chunk() {
        // SQ-0330: the Glk paragraph stylehints on a run's StyleAttrs
        // (Indentation / ParaIndentation / Justification) are carried into the
        // drained transcript chunk's ParaFmt, ready for the wrap to render.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        let attrs = StyleAttrs { indent: Some(4), para_indent: Some(-2), justify: Some(2), ..Default::default() };
        glk.put_text_attr(1, GlkStyle::Normal, StyleColour::default(), attrs, 0, "centered");
        let (_text, chunks) = glk.take_transcript();
        assert_eq!(chunks.len(), 1);
        let para = chunks[0].5;
        assert_eq!(para, crate::state::ParaFmt { indent: 4, para_indent: -2, justify: 2, nowrap_from: None });
    }

    #[test]
    fn grid_colour_flows_to_cells() {
        use zvm::screen::ZColour;
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextGrid);
        glk.window_layout(&[(1, WinType::TextGrid, rect(0, 0, 10, 2), Some(true))]);
        glk.window_tree(Some(leaf(1, WinType::TextGrid, rect(0, 0, 10, 2))));
        let blue_on_white = StyleColour { fg: Some(0x0000_00FF), bg: Some(0x00FF_FFFF), reverse: false };
        glk.grid_put_attr(1, 0, 0, GlkStyle::Normal, blue_on_white, StyleAttrs::default(), 0, "X");
        let cell = glk.screen_model().grid().unwrap().cell(1, 1);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, crate::state::pack_zcolour(ZColour::True24(0x0000_00FF)));
        assert_eq!(cell.bg, crate::state::pack_zcolour(ZColour::True24(0x00FF_FFFF)));
    }

    #[test]
    fn grid_link_flows_to_cells() {
        // A Glk hyperlink value stamped via grid_put_attr must survive onto the
        // neutral GridCell the renderer consumes. (SQ-0258)
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextGrid);
        glk.window_layout(&[(1, WinType::TextGrid, rect(0, 0, 10, 2), Some(true))]);
        glk.window_tree(Some(leaf(1, WinType::TextGrid, rect(0, 0, 10, 2))));
        glk.grid_put_attr(1, 0, 0, GlkStyle::Normal, StyleColour::default(), StyleAttrs::default(), 42, "L");
        glk.grid_put_attr(1, 1, 0, GlkStyle::Normal, StyleColour::default(), StyleAttrs::default(), 0, "x");
        assert_eq!(glk.screen_model().grid().unwrap().cell(1, 1).link, 42, "linked cell carries its link value");
        assert_eq!(glk.screen_model().grid().unwrap().cell(1, 2).link, 0, "an unlinked cell has link 0");
    }

    #[test]
    fn appglk_graphics_fill_composites_into_canvas() {
        let mut g = AppGlk::with_graphics(80, 24, (2, 2), crate::graphics::PictSource::new(None));
        // Simulate a laid-out graphics window id=1 occupying 4x4 cells → 8x8 px.
        g.window_open(1, gvm::glk::WinType::Graphics);
        g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 4, height: 4 }, Some(true))]);
        g.graphics_fill_rect(1, 0x00FF_0000, 0, 0, 8, 8);
        let canvas = g.graphics.get(&1).unwrap();
        assert_eq!(canvas.img.dimensions(), (8, 8));
        assert_eq!(canvas.img.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF]);
    }

    #[test]
    fn screen_model_emits_graphics_leaf() {
        let mut g = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(None));
        g.window_open(1, gvm::glk::WinType::Graphics);
        g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 10, height: 4 }, Some(true))]);
        g.window_tree(Some(leaf(1, WinType::Graphics, rect(0, 0, 10, 4))));
        g.graphics_fill_rect(1, 0x00FF00, 0, 0, 10, 4);
        let model = g.screen_model();
        // The tree's single leaf is a Graphics node for window 1.
        fn find_graphics(n: &crate::engine::WinNode) -> bool {
            match n {
                crate::engine::WinNode::Graphics(_) => true,
                crate::engine::WinNode::Pair { first, second, .. } => find_graphics(first) || find_graphics(second),
                _ => false,
            }
        }
        assert!(find_graphics(&model.root), "graphics window should appear as a Graphics leaf");
    }

    /// Regression (CounterfeitMonkey layout): a game keeps a zero-height
    /// graphics window (win 4) around alongside a real graphics window (win 6,
    /// the image), a status grid, and the text buffer. Every leaf — including the
    /// collapsed one — is part of gvm's tree, so the real graphics window, the
    /// grid, and the buffer all survive the conversion.
    #[test]
    fn screen_model_survives_zero_area_window() {
        use gvm::glk::{Rect, WinType};
        let mut g = AppGlk::with_graphics(80, 24, (9, 19), crate::graphics::PictSource::new(None));
        g.window_open(1, WinType::TextBuffer);
        g.window_open(2, WinType::TextGrid);
        g.window_open(4, WinType::Graphics);
        g.window_open(6, WinType::Graphics);
        g.window_layout(&[
            (1, WinType::TextBuffer, Rect { left: 40, top: 1, width: 40, height: 23 }, Some(true)),
            (2, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }, Some(true)),
            (4, WinType::Graphics, Rect { left: 0, top: 24, width: 80, height: 0 }, Some(true)), // collapsed
            (6, WinType::Graphics, Rect { left: 0, top: 1, width: 40, height: 23 }, Some(true)), // the image
        ]);
        // The matching tree: grid on top; below it a left graphics | right buffer
        // split; and the collapsed zero-height graphics (win 4) at the bottom.
        g.window_tree(Some(vpair(
            1,
            leaf(2, WinType::TextGrid, rect(0, 0, 80, 1)),
            vpair(
                23,
                hpair(
                    40,
                    leaf(6, WinType::Graphics, rect(0, 1, 40, 23)),
                    leaf(1, WinType::TextBuffer, rect(40, 1, 40, 23)),
                ),
                leaf(4, WinType::Graphics, rect(0, 24, 80, 0)),
            ),
        )));
        g.graphics_fill_rect(6, 0x00FF00, 0, 0, 40, 23);
        let model = g.screen_model();

        fn collect(n: &crate::engine::WinNode, out: &mut Vec<(&'static str, u32)>) {
            match n {
                crate::engine::WinNode::Graphics(gw) => out.push(("graphics", gw.win)),
                crate::engine::WinNode::Buffer(_) => out.push(("buffer", 0)),
                crate::engine::WinNode::Grid(_) => out.push(("grid", 0)),
                crate::engine::WinNode::Pair { first, second, .. } => {
                    collect(first, out);
                    collect(second, out);
                }
                crate::engine::WinNode::Blank => {}
                // gvm never produces a v6 layered composite (Phase 1b, zvm-only).
                crate::engine::WinNode::Layered(_) => {}
            }
        }
        let mut leaves = Vec::new();
        collect(&model.root, &mut leaves);
        assert!(
            leaves.iter().any(|&(k, w)| k == "graphics" && w == 6),
            "the real graphics window (win 6) must survive; got {leaves:?}"
        );
        assert!(leaves.iter().any(|&(k, _)| k == "buffer"), "the text buffer must survive; got {leaves:?}");
        assert!(leaves.iter().any(|&(k, _)| k == "grid"), "the status grid must survive; got {leaves:?}");
    }

    #[test]
    fn image_draw_to_buffer_window_records_image_elem() {
        // No Blorb is registered (`PictSource::new(None)`), so the draw is a
        // silent no-op — this asserts the routing path (surrounding Text order
        // survives a buffer-targeted `graphics_draw_image`), not image presence.
        // Resolvable-image coverage comes via Task 5's `glulx_session` test.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // primary buffer
        glk.put_text(1, GlkStyle::Normal, "before\n");
        glk.graphics_draw_image(1, /*resnum*/ 0, /*imagealign*/ 1, 0, None);
        glk.put_text(1, GlkStyle::Normal, "after");
        let elems = glk.take_transcript_elems();
        let kinds: Vec<&str> = elems
            .iter()
            .map(|e| match e {
                crate::session::TranscriptElem::Text { .. } => "T",
                crate::session::TranscriptElem::Image(_) => "I",
                crate::session::TranscriptElem::ScreenClear => "C",
            })
            .collect();
        assert_eq!(kinds.first().copied(), Some("T"));
        assert_eq!(kinds.last().copied(), Some("T"));
    }

    #[test]
    fn take_transcript_elems_coalesces_text_and_keeps_image_between() {
        // Seed the primary buffer log directly (a resolvable Pict needs a Blorb,
        // which the test harness lacks): Text, Text (different style bits), Image,
        // Text. take_transcript_elems must coalesce the two leading Text runs into
        // ONE element carrying TWO run-chunks, keep the Image between, and flush
        // the trailing Text as a final element.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        let pid = glk.primary.expect("primary open");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let log = &mut glk.buffers.get_mut(&pid).unwrap().log;
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, link: 0, para: crate::state::ParaFmt::default(), glk_style: 0, text: "foo".into() });
        log.push(BufElem::Text { bits: 0x02, fg: 0, bg: 0, link: 0, para: crate::state::ParaFmt::default(), glk_style: 0, text: "bar".into() });
        log.push(BufElem::Image(dummy));
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, link: 0, para: crate::state::ParaFmt::default(), glk_style: 0, text: "baz".into() });

        let elems = glk.take_transcript_elems();
        assert_eq!(elems.len(), 3, "coalesced Text, Image, trailing Text");
        match &elems[0] {
            crate::session::TranscriptElem::Text { text, runs } => {
                assert_eq!(text, "foobar", "the two leading runs coalesce into one element");
                assert_eq!(runs.len(), 2, "each source run kept as its own chunk, in order");
                assert_eq!(runs[0], (3, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, crate::state::ParaFmt::default(), 0, false));
                assert_eq!(runs[1].0, 3);
                assert_eq!(runs[1].1, 0x02, "second chunk carries the different style bits");
            }
            _ => panic!("elems[0] must be Text"),
        }
        assert!(matches!(&elems[1], crate::session::TranscriptElem::Image(_)), "image sits between");
        match &elems[2] {
            crate::session::TranscriptElem::Text { text, .. } => assert_eq!(text, "baz"),
            _ => panic!("trailing Text must flush as a final element"),
        }
    }

    #[test]
    fn image_draw_to_graphics_window_still_hits_canvas() {
        // A graphics-window draw must NOT push a buffer image elem; it updates
        // a Canvas via the existing graphics path.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(5, WinType::Graphics);
        glk.graphics_draw_image(5, 0, 10, 10, None);
        // No primary buffer is open → elems empty.
        assert!(glk.take_transcript_elems().is_empty());
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn graphics_draw_image_reports_false_when_pict_missing() {
        // SQ-0175 part A: with no Blorb registered, `resnum` never resolves,
        // so the backend must report false rather than always claiming success.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        assert!(!glk.graphics_draw_image(1, 0, 1, 0, None), "buffer window, missing image");

        glk.window_open(5, WinType::Graphics);
        assert!(!glk.graphics_draw_image(5, 0, 10, 10, None), "graphics window, missing image");
    }

    #[test]
    fn graphics_draw_image_reports_true_when_pict_resolves() {
        // A resnum backed by a real, decodable Pict in the Blorb must report
        // true on both the buffer-window and graphics-window draw paths.
        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes());
        let mut glk = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(Some(blorb)));
        glk.window_open(1, WinType::TextBuffer);
        assert!(glk.graphics_draw_image(1, /*resnum*/ 1, /*imagealign*/ 1, 0, None), "buffer window, resolvable image");

        let blorb2 = crate::graphics::test_blorb_with_pict(1, &png_bytes());
        let mut glk2 = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(Some(blorb2)));
        glk2.window_open(5, WinType::Graphics);
        assert!(glk2.graphics_draw_image(5, /*resnum*/ 1, 10, 10, None), "graphics window, resolvable image");
    }

    #[test]
    fn appglk_schannel_create_allocates_refs_and_rocks() {
        use gvm::glk::GlkBackend;
        let mut g = AppGlk::new(80, 24);
        let a = g.schannel_create(11);
        let b = g.schannel_create(22);
        assert_ne!(a, 0, "a created channel has a nonzero ref");
        assert_ne!(a, b, "distinct channels get distinct refs");
        assert_eq!(g.schannel_get_rock(a), 11);
        assert_eq!(g.schannel_get_rock(b), 22);
        assert_eq!(g.schannel_get_rock(9999), 0, "unknown channel → rock 0");
        // iterate: 0 → first, then next, then 0 at the end.
        let (first, first_rock) = g.schannel_iterate(0);
        assert_eq!((first, first_rock), (a, 11));
        let (second, second_rock) = g.schannel_iterate(first);
        assert_eq!((second, second_rock), (b, 22));
        assert_eq!(g.schannel_iterate(second), (0, 0), "past the end → (0,0)");
    }

    #[test]
    fn appglk_schannel_ops_buffer_in_order_with_volume_snapshot() {
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0);
        g.schannel_set_volume(c, 0x8000);          // half volume
        g.schannel_play(c, 5, 3, 9);               // play_ext(chan, snd, repeats, notify)
        g.schannel_stop(c);
        g.schannel_destroy(c);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![
            SchannelOp::SetVolume { chan: c, vol: 0x8000 },
            SchannelOp::Play { chan: c, snd: 5, repeats: 3, notify: 9, volume: 0x8000, paused: false },
            SchannelOp::Stop { chan: c },
            SchannelOp::Destroy { chan: c },
        ]);
        assert!(g.take_sound_ops().is_empty(), "draining clears the buffer");
        assert_eq!(g.schannel_get_rock(c), 0, "destroy removed the channel");
    }

    #[test]
    fn appglk_play_snapshots_default_full_volume() {
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0); // no set_volume → default 0x10000 (Glk full)
        g.schannel_play(c, 1, 1, 0);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![SchannelOp::Play { chan: c, snd: 1, repeats: 1, notify: 0, volume: 0x10000, paused: false }]);
    }

    #[test]
    fn appglk_pause_on_empty_channel_snapshots_paused_into_next_play() {
        // Glk 0.7.3 §8.3: pausing a channel while it is empty must make a
        // subsequently-played sound start paused; unpause releases it. The
        // paused state is snapshotted into the Play op (like volume), so the
        // player (which cannot see AppGlk) starts the sink paused.
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0);
        g.schannel_pause(c); // pause the empty channel
        g.schannel_play(c, 1, 1, 0); // this sound must start paused
        assert_eq!(
            g.take_sound_ops(),
            vec![
                SchannelOp::Pause { chan: c },
                SchannelOp::Play { chan: c, snd: 1, repeats: 1, notify: 0, volume: 0x10000, paused: true },
            ],
        );
        // Unpause clears it, so a later play starts unpaused again.
        g.schannel_unpause(c);
        g.schannel_play(c, 2, 1, 0);
        assert_eq!(
            g.take_sound_ops(),
            vec![
                SchannelOp::Unpause { chan: c },
                SchannelOp::Play { chan: c, snd: 2, repeats: 1, notify: 0, volume: 0x10000, paused: false },
            ],
        );
    }

    #[test]
    fn appglk_sound2_create_ext_seeds_volume_and_new_ops_buffer() {
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        // create_ext seeds the channel volume, so a later play snapshots it.
        let c = g.schannel_create_ext(4, 0x4000); // quarter volume
        assert_ne!(c, 0, "create_ext returns a nonzero channel ref");
        assert_eq!(g.schannel_get_rock(c), 4, "create_ext stores the rock");
        g.schannel_play(c, 2, 1, 0);
        g.schannel_pause(c);
        g.schannel_unpause(c);
        g.schannel_set_volume_ext(c, 0x8000, 500, 7);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![
            SchannelOp::Play { chan: c, snd: 2, repeats: 1, notify: 0, volume: 0x4000, paused: false },
            SchannelOp::Pause { chan: c },
            SchannelOp::Unpause { chan: c },
            SchannelOp::SetVolumeExt { chan: c, vol: 0x8000, duration_ms: 500, notify: 7 },
        ]);
    }
}

#[cfg(all(test, feature = "t-session"))]
mod heading_tests {
    use super::*;
    use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, WinType};

    fn primary_backend() -> AppGlk {
        let mut b = AppGlk::new(80, 24);
        b.window_open(1, WinType::TextBuffer);
        b.window_layout(&[(1, WinType::TextBuffer, GlkRect { left: 0, top: 0, width: 80, height: 24 }, Some(true))]);
        b
    }

    // Feed a run via the colourless trait entry (delegates to put_text_attr).
    fn put(b: &mut AppGlk, style: GlkStyle, s: &str) {
        b.put_text(1, style, s);
    }

    #[test]
    fn subheader_line_is_the_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Studio Apartment\n");
        put(&mut b, GlkStyle::Normal, "You climb out of bed.\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Studio Apartment"));
        // Drained: a second call with no new heading is None.
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn last_subheader_wins_over_banner_title() {
        // The heading ends its own line — Inform's room description heading rule is
        // `[bold type][printed name][roman type]` and the body text follows a paragraph
        // break, so the description never shares the line. The fixture used to run the
        // two together, which made it indistinguishable from the banner above it once
        // `line_rest_disqualifies` existed to tell them apart (SQ-1285).
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Coloratura");
        put(&mut b, GlkStyle::Normal, " by lynnea glasser\n");
        put(&mut b, GlkStyle::Subheader, "Inside the Cellarium\n");
        put(&mut b, GlkStyle::Normal, "A white structure.\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Inside the Cellarium"));
    }

    #[test]
    fn emphasized_and_header_are_not_headings() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Header, "Superluminal Vagrant Twin\n");
        put(&mut b, GlkStyle::Emphasized, "Knock.");
        put(&mut b, GlkStyle::Normal, "Prose.\n");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn menu_only_normal_text_has_no_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "1) Yes\n2) No\n");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn heading_char_by_char_runs_accumulate() {
        // Games often emit one glk_put_char per character.
        let mut b = primary_backend();
        for ch in "War Chest".chars() {
            put(&mut b, GlkStyle::Subheader, &ch.to_string());
        }
        put(&mut b, GlkStyle::Normal, "\nThe battle.\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("War Chest"));
    }

    #[test]
    fn mid_line_subheader_is_an_inline_link_not_a_room() {
        // Superluminal Vagrant Twin renders command hints ("credits", "land") as
        // Subheader mid-line, before and after the real room heading. Only the
        // line-start heading counts; a trailing inline link must NOT overwrite it.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "(Type ");
        put(&mut b, GlkStyle::Subheader, "credits"); // inline link, mid-line
        put(&mut b, GlkStyle::Normal, " to learn who made this.)\n\n");
        put(&mut b, GlkStyle::Subheader, "Orbiting Boony"); // room heading, line start
        put(&mut b, GlkStyle::Normal, "\nA grey world. You going to ");
        put(&mut b, GlkStyle::Subheader, "land"); // inline link, mid-line
        put(&mut b, GlkStyle::Normal, " soon?\n\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Orbiting Boony"));
    }

    #[test]
    fn window_clear_resets_line_start_for_next_heading() {
        // A game that clears the screen mid-line and then prints the room title
        // with no leading newline: the clear returns the cursor to line start, so
        // the heading must still be recognized.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "loading"); // leaves at_line_start = false
        b.window_clear(1);
        put(&mut b, GlkStyle::Subheader, "Grand Hall"); // top of cleared window
        put(&mut b, GlkStyle::Normal, "\nA vast chamber.\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Grand Hall"));
    }

    #[test]
    fn a_banner_on_a_press_any_key_page_is_not_a_room() {
        // THE BAT's title page: an act list on its own line, a blank line, and then the
        // "press any key" note. Nothing about the words says it is not a room — the blank
        // line and the keypress prompt do. (SQ-0732)
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Prologue • ACT I • Interlude • Epilogue");
        put(&mut b, GlkStyle::Normal, " \n\n[Please press any key to continue.]\n");
        assert_eq!(b.take_room_heading(false), None);
    }

    #[test]
    fn a_heading_joined_to_its_description_is_a_room_even_before_a_keypress() {
        // Half the discriminator on its own would be wrong: a room the player really did
        // walk into can be followed by a cutscene that ends on a keypress.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Master's Bedroom\n");
        put(&mut b, GlkStyle::Normal, "To the west, you could enter the upstairs corridor.\n");
        put(&mut b, GlkStyle::Normal, "\n[Please press any key to continue.]\n");
        assert_eq!(b.take_room_heading(false).as_deref(), Some("Master's Bedroom"));
    }

    #[test]
    fn a_detached_heading_is_a_room_at_the_command_prompt() {
        // Adventure in `superbrief`: heading, blank line, object list, command prompt. The
        // other half of the discriminator on its own would throw this room away.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Inside Building\n");
        put(&mut b, GlkStyle::Normal, "\nThere are some keys on the ground here.\n\n>");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Inside Building"));
    }

    #[test]
    fn a_superbrief_room_is_a_heading_a_blank_line_and_the_prompt() {
        // An empty superbrief room prints nothing but its name. The read prompt the game
        // leaves behind is not prose, so the heading still stands even on a keypress page.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "At End Of Road\n");
        put(&mut b, GlkStyle::Normal, "\n>");
        assert_eq!(b.take_room_heading(false).as_deref(), Some("At End Of Road"));
    }

    #[test]
    fn a_detached_heading_on_a_page_that_reads_a_line_without_prompting_is_not_a_room() {
        // cragne Manor's opening page. It is textually the same shape as a superbrief
        // room -- own-line heading, blank line, more text -- and it ends at Glk LINE
        // input, so "the turn ended at a keypress" never fires. What it never does is
        // print the parser's command prompt: the game reads the answer itself. (SQ-0733)
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "CONTENT WARNING");
        put(&mut b, GlkStyle::Normal, "\n\nPlease be warned that this game contains:\n\n");
        put(&mut b, GlkStyle::Normal, "cosmic horror, body horror, gore, violence.\n\n");
        put(&mut b, GlkStyle::Normal, "Would you still like to continue? (Please type yes or no.)\n");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn a_detached_heading_followed_by_prose_is_a_room_when_the_prompt_follows() {
        // The same page shape, ended by the command prompt, is a room -- what rejects
        // cragne's warning is the missing prompt, not the length of what follows the
        // blank line. A room walked into mid-cutscene looks exactly like this.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Railway Platform");
        put(&mut b, GlkStyle::Normal, "\n\nYou stand on a platform. There is a wooden bench, ");
        put(&mut b, GlkStyle::Normal, "a storage locker, and a vending machine.\n\n>");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Railway Platform"));
    }

    #[test]
    fn a_cleared_window_carries_its_read_prompt_away() {
        // A prompt printed before a `glk_window_clear` is no longer on screen, so it
        // cannot vouch for a heading printed into the fresh window.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "Anything at all.\n\n>");
        b.window_clear(1);
        put(&mut b, GlkStyle::Subheader, "CONTENT WARNING");
        put(&mut b, GlkStyle::Normal, "\n\nType yes or no.\n");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn an_earlier_room_survives_a_banner_printed_after_it() {
        // A turn that walks into a room and then ends on an act-break page keeps the room:
        // the banner is rejected on its own, not by clearing what came before it.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Master's Bedroom\n");
        put(&mut b, GlkStyle::Normal, "To the west, the upstairs corridor.\n\n");
        put(&mut b, GlkStyle::Subheader, "– Interlude –\n");
        put(&mut b, GlkStyle::Normal, "\nThe guests are arriving.\n");
        assert_eq!(b.take_room_heading(false).as_deref(), Some("Master's Bedroom"));
    }

    #[test]
    fn a_bolded_object_name_opening_a_take_listing_is_not_a_room() {
        // SQ-1285. Counterfeit Monkey's HIGHLIGHT option (`boldening`) prints every
        // object name in bold type, which Glk carries as `Subheader`. Its `get all`
        // listing therefore opens each line with a bold noun, and the turn ends at the
        // parser's own command prompt — everything the old rule asked for. The room the
        // player is standing in is the Midway; "ear" is a severed ear in their hands.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "ale");
        put(&mut b, GlkStyle::Normal, ": We acquire the ");
        put(&mut b, GlkStyle::Subheader, "ale");
        put(&mut b, GlkStyle::Normal, ".\n");
        put(&mut b, GlkStyle::Subheader, "ear");
        put(&mut b, GlkStyle::Normal, ": We take the ");
        put(&mut b, GlkStyle::Subheader, "ear");
        put(&mut b, GlkStyle::Normal, ".\n\n>");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn a_bolded_object_name_opening_a_paragraph_is_not_a_room() {
        // The same bolding opens ordinary prose all over that game — an initial
        // appearance is a paragraph, so its first word sits at line start too.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "The Aquarium Bookstore");
        put(&mut b, GlkStyle::Normal, " is to the east. It's dim inside.\n\n>");
        assert_eq!(b.take_room_heading(true), None);
    }

    #[test]
    fn a_heading_keeps_its_roman_parenthetical() {
        // What may share the heading's line: the library's "(on the chair)", printed in
        // roman after the bold name. That is still a room.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Studio Apartment");
        put(&mut b, GlkStyle::Normal, " (on the bed)\n");
        put(&mut b, GlkStyle::Normal, "You climb out of bed.\n");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Studio Apartment"));
    }

    #[test]
    fn a_room_bolded_by_the_same_option_still_reads_as_a_room() {
        // Non-vacuity for the two rejections above: with HIGHLIGHT on, Counterfeit
        // Monkey's own room description bolds every noun in it, and only the heading
        // owns its line. The room must still be found.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Midway\n");
        put(&mut b, GlkStyle::Normal, "Here in front of the ");
        put(&mut b, GlkStyle::Subheader, "pharmacy");
        put(&mut b, GlkStyle::Normal, ", various contests have been set up.\n\n>");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Midway"));
    }

    #[test]
    fn an_earlier_room_survives_a_sentence_that_opens_with_a_bolded_name() {
        // Rejecting a candidate must not take the heading already confirmed this turn
        // with it: the walk into the room comes first, its bolded description after.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Midway\n");
        put(&mut b, GlkStyle::Normal, "Contests have been set up.\n\n");
        put(&mut b, GlkStyle::Subheader, "The barker");
        put(&mut b, GlkStyle::Normal, " is holding a tube.\n\n>");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Midway"));
    }

    #[test]
    fn a_bolded_name_on_the_line_below_does_not_take_the_heading_with_it() {
        // SQ-1295. The case above has a blank line between the heading and the bolded
        // sentence, so the heading is confirmed by the description before the sentence
        // is even seen. Counterfeit Monkey's Brown's Lab has NO such line: the NPC's
        // bolded name is the first thing on the line directly below the heading.
        //
        // That candidate was one character from confirmation, and the character that
        // arrived opened a new `Subheader` run — which used to start a fresh heading
        // run, leaving "Brown's Lab" to be overwritten by `finalize_heading` and then
        // thrown away with "Professor Brown" when `line_rest_disqualifies` rejected it.
        // The turn reported NO room at all, which cost the map the room's name and, one
        // layer up, the Glulx room lock (see `app::glulx_roomlock`).
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Brown's Lab\n");
        put(&mut b, GlkStyle::Subheader, "Professor Brown");
        put(&mut b, GlkStyle::Normal, ", the Reification of Abstracts researcher, is hunched over his work table.\n\n>");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Brown's Lab"));
    }

    /// A single-leaf `WinTree` for this module (the other test module has its own).
    fn win_leaf(id: u32, wintype: WinType, width: u32, height: u32) -> WinTree {
        WinTree::Leaf {
            id,
            wintype,
            rect: GlkRect { left: 0, top: 0, width, height },
            bg: None,
            fg: None,
            reverse: false,
        }
    }

    /// Every character the grid windows of `b`'s screen model hold, row by row.
    fn grid_text(b: &AppGlk) -> String {
        fn walk(n: &crate::engine::WinNode, out: &mut String) {
            match n {
                crate::engine::WinNode::Pair { first, second, .. } => {
                    walk(first, out);
                    walk(second, out);
                }
                crate::engine::WinNode::Grid(g) => {
                    for row in 1..=g.rows {
                        for col in 1..=g.cols {
                            out.push(g.cell(row, col).ch);
                        }
                        out.push('\n');
                    }
                }
                _ => {}
            }
        }
        let mut out = String::new();
        walk(&b.screen_model().root, &mut out);
        out
    }

    #[test]
    fn a_display_snapshot_puts_back_what_a_silent_question_wrote() {
        // SQ-1293. `GlulxSession::silent_look` types a command nobody asked for and
        // restores the VM afterwards — but the VM is only half the game's state. The
        // backend keeps what every window CONTAINS and the app renders from that, so
        // the question has to be undone here too. Every assertion below names one
        // thing a VM-only restore leaves behind.
        let mut b = primary_backend();
        b.window_open(2, WinType::TextGrid);
        let two_windows = [
            (2, WinType::TextGrid, GlkRect { left: 0, top: 0, width: 80, height: 1 }, Some(true)),
            (1, WinType::TextBuffer, GlkRect { left: 0, top: 1, width: 80, height: 23 }, Some(true)),
        ];
        b.window_layout(&two_windows);
        b.window_tree(Some(win_leaf(2, WinType::TextGrid, 80, 1)));
        b.grid_put(2, 0, 0, GlkStyle::Normal, "Maze                Score: 0");
        put(&mut b, GlkStyle::Subheader, "Maze\n");
        put(&mut b, GlkStyle::Normal, "You are in a maze.\n\n>");
        // The real turn drains its own output and reads its own room, exactly as
        // `finish_turn` does before the question is asked.
        let _ = b.take_transcript_elems();
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Maze"), "the real turn's own room");
        let before_dump = b.window_dump_lines();
        let before_grid = grid_text(&b);

        // Ask the question: more prose, a rewritten status line, a window opened
        // while answering, and a heading that must not stand in for the next turn's.
        let snap = b.display_snapshot();
        put(&mut b, GlkStyle::Normal, "look\n"); // the parser's echo of the question
        put(&mut b, GlkStyle::Subheader, "Cavern\n");
        put(&mut b, GlkStyle::Normal, "A dripping cave.\n\n>");
        b.grid_put(2, 0, 0, GlkStyle::Normal, "Cavern              Score: 9");
        assert_eq!(b.take_room_heading(true).as_deref(), Some("Cavern"), "the answer, read once");
        b.window_open(3, WinType::TextBuffer);
        b.window_tree(Some(win_leaf(3, WinType::TextBuffer, 80, 24)));
        b.restore_display_snapshot(snap);

        // The status line the question rewrote.
        assert_eq!(grid_text(&b), before_grid, "the grid is back");
        // The window it opened, and the tree the app lays out from.
        assert_eq!(b.window_dump_lines(), before_dump, "so is the window tree");
        // The buffer log, which is what a VM-only restore leaves growing: the drain
        // pointer moves, the text does not, so the question's prose is owed to the
        // player's transcript on the NEXT turn and prints the room description twice.
        assert!(
            b.take_transcript_elems().is_empty(),
            "the question's prose is owed to nobody, and a moved drain pointer over an \
             un-rewound log would owe the player the room description a second time"
        );
        // And the heading scan, so the next turn is read the way the real one left it.
        assert_eq!(
            b.take_room_heading(true),
            None,
            "the answer must not stand in for the next turn's room"
        );
    }

    #[test]
    fn two_stacked_banners_do_not_promote_the_upper_one() {
        // The other half of the rule, and the reason the verdict cannot be reached when
        // the displacing run OPENS: a banner page stacks own-line `Subheader` lines, and
        // the upper one is no more a room than the lower (THE BAT's title page, SQ-0732).
        // What separates the two shapes is whether the displacing line turns out to be
        // prose opened by a bolded noun, or a line the newcomer owns outright.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "THE BAT\n");
        put(&mut b, GlkStyle::Subheader, "An Interactive Nightmare\n");
        put(&mut b, GlkStyle::Normal, "\nPress any key to begin.\n");
        assert_eq!(b.take_room_heading(false), None);
    }

    #[test]
    fn a_styled_word_opening_a_sentence_is_not_a_room() {
        // Kerkerkruip renders the "Enable" of "Enable the screen reader mode?" as a
        // Subheader hyperlink — at line start, so the line-start rule alone accepted it.
        // The paragraph break below it and the keypress prompt reject it.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Enable");
        put(&mut b, GlkStyle::Normal, " the screen reader mode? Please enter: Yes or No\n");
        put(&mut b, GlkStyle::Normal, "\nThis option can be changed later from the menu.\n");
        assert_eq!(b.take_room_heading(false), None);
    }
}
