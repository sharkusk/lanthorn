//! Engine abstraction (Glulx 3b-i).
//!
//! The app talks to a running game through the engine-neutral [`Engine`] trait
//! and a small family of app-owned, engine-agnostic types ([`KeyInput`],
//! [`ScreenModel`], [`Introspect`], the reserved [`Debugger`], and the
//! engine-tagged [`EngineSave`]).  `zvm`'s `GameSession` implements `Engine`
//! (see `session.rs`); a future `gvm` (Glulx) session will slot in beside it.
//!
//! These types deliberately carry **no** `Glk` / `Glulx` / `Z-machine` specifics
//! in their public surface: a `GridWindow` is a grid of style-bit cells, a
//! status line is a location plus a score/turns or clock field, an object
//! handle is an opaque `u16`.  Each engine adapts its own world into them.

use std::any::Any;
use std::collections::BTreeMap;

use crate::session::{FilenameReq, InputKind, TurnResult};

/// What an object is and what it can be called, re-exported so a caller of
/// [`Introspect`] names it without depending on `grammar-model` directly.
/// [`Adjectives`] travels with it: it says whether the story could be asked for
/// an object's adjectives at all, which is a different claim from having none.
pub use grammar_model::{Adjectives, ObjectWordSet, ObjectWords};

/// What a room's own exit table declares for one direction (SQ-1257),
/// re-exported so a caller of [`Engine::declared_exit`] names it without
/// depending on `zvm` directly — the same reason [`ObjectWords`] travels
/// through this module above.
pub use zvm::world::DeclaredExit;

// ── Neutral key input ───────────────────────────────────────────────────────

/// A neutral, terminal-agnostic key press.
///
/// The app maps a crossterm `KeyEvent` into this with [`key_event_to_input`];
/// each engine converts it into its own input encoding (the `zvm` adapter maps
/// it to ZSCII; a Glk adapter would map it to Glk keycodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    /// Function key F1..=F12 (carries the digit, e.g. `Func(1)` for F1).
    Func(u8),
}

/// Map a crossterm `KeyEvent` to a neutral [`KeyInput`].
///
/// Returns `None` for keys with no neutral representation (media keys, modifier
/// presses, etc.).  Modifiers are not encoded here — the caller decides whether
/// a Ctrl/Alt combo is app routing or game input before forwarding.
pub fn key_event_to_input(key: crossterm::event::KeyEvent) -> Option<KeyInput> {
    use crossterm::event::KeyCode;
    Some(match key.code {
        KeyCode::Char(c) => KeyInput::Char(c),
        KeyCode::Enter => KeyInput::Enter,
        KeyCode::Backspace => KeyInput::Backspace,
        KeyCode::Tab => KeyInput::Tab,
        KeyCode::Esc => KeyInput::Escape,
        KeyCode::Up => KeyInput::Up,
        KeyCode::Down => KeyInput::Down,
        KeyCode::Left => KeyInput::Left,
        KeyCode::Right => KeyInput::Right,
        KeyCode::Home => KeyInput::Home,
        KeyCode::End => KeyInput::End,
        KeyCode::PageUp => KeyInput::PageUp,
        KeyCode::PageDown => KeyInput::PageDown,
        KeyCode::Delete => KeyInput::Delete,
        KeyCode::Insert => KeyInput::Insert,
        KeyCode::F(n) => KeyInput::Func(n),
        _ => return None,
    })
}

// ── Neutral screen model (window tree) ──────────────────────────────────────

/// One styled character cell in a [`GridWindow`].
///
/// `style` is a neutral text-style bitset following the common interactive-
/// fiction convention (bit 1 = reverse, 2 = bold, 4 = italic, 8 = fixed-pitch).
#[derive(Debug, Clone, Copy)]
pub struct GridCell {
    pub ch: char,
    pub style: u8,
    /// Packed foreground colour (see `crate::state::pack_zcolour`); 0 = Default.
    pub fg: u32,
    /// Packed background colour; 0 = Default.
    pub bg: u32,
    /// Glk hyperlink value stamped on this cell (0 = not a link). (SQ-0258)
    pub link: u32,
    /// Glk style class (0=Normal .. 10=User2) for the theme's per-style colour
    /// slot (SQ-0331). Z-machine grid cells are always Normal (0).
    pub glk_style: u8,
}

impl Default for GridCell {
    fn default() -> Self {
        GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }
    }
}

/// A text-grid window: fixed-size positioned cells with a cursor (a status line
/// or a Glk text-grid).  The renderer applies a viewport over the logical grid
/// and auto-follows the cursor.
/// A grid window's border-presence preference (SQ-0286). Only a Glulx window
/// split carries an explicit preference; the Z-machine, the default constructor,
/// and a parentless Glulx root leave it `Unspecified` so the theme decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderPref {
    /// No border preference (Z-machine, or a parentless Glulx root): the theme decides.
    #[default]
    Unspecified,
    /// A Glulx split explicitly requested a border (`winmethod_Border`). Presence forced on.
    Border,
    /// A Glulx split requested `winmethod_NoBorder`. Presence forced off.
    NoBorder,
}

/// An `erase_window`'s surviving background fill (SQ-0584): the packed colour it
/// painted (0 = the page default) and its draw-order stamp, so several fills — and
/// the picture draws they interleave with — composite in the order the game made
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErasedFill {
    pub bg: u32,
    pub seq: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GridWindow {
    /// This window's Glk id (0 for a Z-machine/Scott grid, which has no Glk
    /// identity). Lets the renderer record which drawn rect belongs to which
    /// window for mouse/hyperlink hit-testing (SQ-1203) without re-deriving it
    /// from gvm's own (possibly gutter-skewed) layout.
    pub win: u32,
    /// Logical grid width in columns.
    pub cols: u16,
    /// Logical grid height in rows (allocation height).
    pub rows: u16,
    /// `rows * cols` cells in row-major order.
    pub cells: Vec<GridCell>,
    /// Active row count to render (e.g. the Z-machine `upper_window_rows`); may
    /// be less than `rows`.
    pub active_rows: u16,
    /// 1-based cursor (row, col).
    pub cursor: (u16, u16),
    /// True when this grid is the engine's currently selected output window
    /// (drives whether the cursor is shown while awaiting a keypress).
    pub cursor_active: bool,
    /// The game's border-presence preference (SQ-0286). `Unspecified` (the
    /// default) lets the theme decide; a Glulx split forces `Border`/`NoBorder`.
    pub border: BorderPref,
    /// This window's own Normal-style background colour (packed RGB
    /// `0x00RRGGBB`), or `None` if the game set none (the host uses its theme).
    pub bg: Option<u32>,
    /// This window's own Normal-style foreground colour (packed RGB), or `None`.
    pub fg: Option<u32>,
    /// The grid's Normal-style ReverseColor flag: when the game reversed the grid
    /// styles with no explicit colours (Counterfeit Monkey's menu), the empty-cell
    /// fill is drawn reversed too, so the whole window matches. (SQ-0403)
    pub reverse: bool,
    /// This window was ERASED more recently than the story last printed, so its rect
    /// is an opaque field of this packed background colour (0 = the page default) —
    /// v6 only (SQ-0584). ZMSD §8.8.5.3: erasing a window fills its rect with the
    /// window's background, and on a real interpreter that paint sits on the one
    /// shared screen bitmap, hiding whatever was under it. That is what makes
    /// advent.z6's `help` menu a solid panel rather than text floating over the
    /// story. `None` — the ordinary case — means the story text is the newer paint,
    /// so nothing is covered.
    pub fill: Option<ErasedFill>,
    /// Pixel-positioned text runs (v6 only; empty for Glulx/cell grids). Each is
    /// the exact 1-based window-relative pixel start the game printed at, so the
    /// pixel raster draws status text where the game placed it (Zork Zero puts
    /// its banner text at rows 6/14, ON the ribbon art) instead of snapping to
    /// the char grid. The `cells` grid remains the cell-mode fallback.
    pub px_texts: Vec<PxText>,
}

/// One pixel-positioned text run in a v6 grid window (see `GridWindow::px_texts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PxText {
    pub y: u16,
    pub x: u16,
    pub text: String,
    /// Z-machine style bits (1=reverse, 2=bold, 4=italic, 8=fixed).
    pub style: u8,
    /// Packed ZColour (`state::pack_zcolour`); 0 = default.
    pub fg: u32,
    pub bg: u32,
    /// The screen character CELL this run's first glyph was written at, 0-based —
    /// `zvm::screen::V6Text::grow`/`gcol`, carried through unchanged.
    ///
    /// A cell backend places by this and never by `(x - 1) / cell_width`. The
    /// division is the column only while the pen advances exactly one declared
    /// cell per character; on Arthur's Amiga press it advances ~10.4 native pixels
    /// against a declared 8, so the quotient climbs 1.3 per glyph and a derived
    /// column skips cells — `Churchyard` reads `Ch urc  hy ard`, worse the wider
    /// the pane (SQ-1009). The engine keeps a dense grid beside the pixel runs;
    /// this is that grid's answer, and for every fixed-pen machine it is exactly
    /// what the division gave.
    pub grow: u16,
    pub gcol: u16,
}

impl PxText {
    /// A run whose grid cell is the DERIVATION `((y-1)/h, (x-1)/w)`.
    ///
    /// Exactly what a fixed-pen machine emits — the pen and the declared cell
    /// agree there — so it is what a fixture placing paint by hand means, and what
    /// a caller synthesising a run from pixels alone can honestly say.
    pub fn derived(
        y: u16,
        x: u16,
        text: String,
        style: u8,
        fg: u32,
        bg: u32,
        cell: zvm::screen::V6Cell,
    ) -> PxText {
        PxText { y, x, text, style, fg, bg, grow: cell.row_of(y), gcol: cell.col_of(x) }
    }
}

impl GridWindow {
    /// Resize to `rows` × `cols`, clearing all cells.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![GridCell::default(); rows as usize * cols as usize];
    }

    /// Cell at 1-based (`row`, `col`), or a blank default when out of bounds.
    pub fn cell(&self, row: u16, col: u16) -> GridCell {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return GridCell::default();
        }
        let idx = ((row - 1) as usize) * self.cols as usize + (col - 1) as usize;
        self.cells.get(idx).copied().unwrap_or_default()
    }

    /// Write `ch`/`style` at 1-based (`row`, `col`).  Out-of-bounds is a no-op.
    pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8) {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return;
        }
        let idx = ((row - 1) as usize) * self.cols as usize + (col - 1) as usize;
        if let Some(c) = self.cells.get_mut(idx) {
            *c = GridCell { ch, style, fg: 0, bg: 0, link: 0, glk_style: 0 };
        }
    }
}

/// A text-buffer window: the scrolling, wrapped, styled lower window.
///
/// For the Z-machine (and a Glulx game's **primary** window) the app keeps its
/// own transcript buffer, so [`primary`](Self::primary) is set and `lines`/`runs`
/// stay empty — the renderer draws this window from `state.transcript` (keeping
/// search / persistence / styling). A Glulx layout's **extra** buffer windows
/// set `primary = false` and carry their inline content in `lines`/`runs`/`scroll`.
#[derive(Debug, Clone, Default)]
pub struct BufferWindow {
    /// This window's Glk id (0 for a Z-machine/Scott buffer, which has no Glk
    /// identity). Lets the renderer record which drawn rect belongs to which
    /// window for mouse/hyperlink hit-testing (SQ-1203) without re-deriving it
    /// from gvm's own (possibly gutter-skewed) layout.
    pub win: u32,
    /// Accumulated logical lines (split on `\n`) for an inline (non-primary)
    /// buffer window. Empty for the primary window.
    pub lines: Vec<String>,
    /// Per-line style runs, parallel to [`lines`](Self::lines).
    pub runs: Vec<Vec<crate::state::StyleRun>>,
    /// Per-line Glk paragraph layout, parallel to [`lines`](Self::lines) (SQ-0330).
    pub para: Vec<crate::state::ParaFmt>,
    /// Optional inline image parallel to `lines` (always same length). `Some`
    /// marks a line that renders as an image band instead of text.
    pub images: Vec<Option<crate::inline_image::InlineImage>>,
    /// Scrollback offset (0 = newest at bottom).
    pub scroll: u16,
    /// True when this is the primary window whose content the app mirrors into
    /// `state.transcript`; the renderer then draws it via the transcript path.
    pub primary: bool,
    /// This window's own Normal-style background colour (packed RGB
    /// `0x00RRGGBB`), or `None` if the game set none (the host uses its theme).
    pub bg: Option<u32>,
    /// This window's own Normal-style foreground colour (packed RGB), or `None`.
    pub fg: Option<u32>,
    /// True for a chrome panel (e.g. the Scott room panel) drawn with the themed
    /// `scott_room_panel` colour instead of the transcript colour, so the top and
    /// bottom of a split read as distinct regions. A game-set `bg` still wins.
    pub panel: bool,
    /// Where the prose this window has streamed is currently SITTING on the v6
    /// screen (SQ-0729), as absolute pixel runs — zvm's `ZWindow::streamed`.
    ///
    /// Live screen state, not history: the game's `erase_window` empties it and a
    /// scroll drops what leaves the top, so it is what a real interpreter would
    /// have on the glass right now. The host transcript remains the source of this
    /// window's prose for every ordinary game and this stays unread; it exists for
    /// the one shape where the transcript is the wrong reading of the window — a
    /// story window whose own art ENCLOSES it, which is a canvas the game paints
    /// into rather than a page it narrates on. Empty for non-v6 engines.
    pub px_runs: Vec<crate::engine::PxText>,
    /// True when the player is typing INTO this window — the game is reading input
    /// through it (SQ-0746). Only a non-primary buffer ever sets it: the primary
    /// window's live input line rides the transcript and is drawn with it.
    ///
    /// A v6 game may read through a display panel it has declared is not the
    /// transcript (fmvpoker's bet and quit prompts), and the host echo has to follow
    /// the read: it belongs after that window's own prompt, not in a story window
    /// the player is not typing into. Always false for other engines.
    pub reads_input: bool,
}

/// Which leaf kind a recorded drawn rect ([`crate::render::screen::StoryPaneMetrics::win_rects`])
/// belongs to. Engine-neutral (unlike gvm's own `WinType`, which stays inside
/// the Glulx adapter per the architecture rule that Glk never leaks into
/// shared app types) — it exists only to pick the right coordinate space
/// (cells vs. pixels) when hit-testing a click against the DRAWN rect (SQ-1203).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinKind {
    Grid,
    Buffer,
    Graphics,
}

/// How a [`WinNode::Pair`] divides its space.
#[derive(Debug, Clone, Copy, Default)]
pub struct Split {
    /// Size (rows or cols, per `vertical`) given to the first child.
    pub fixed: u16,
}

/// A graphics-window leaf: a snapshot of the window's canvas for rendering.
#[derive(Debug, Clone)]
pub struct GraphicsWindow {
    pub win: u32,
    pub canvas: std::sync::Arc<image::RgbaImage>,
    pub version: u64,
    /// Scale the canvas up to fill the window (preserving aspect), rather than
    /// centering it at native size. Set for small pixel-art canvases like Scott
    /// Adams room pictures (256×96); Glulx keeps native-size centering.
    pub upscale: bool,
}

/// One window placed at an absolute cell rect within the story pane, for the
/// v6 z-ordered layered composite (Phase 1b). `x`/`y`/`w`/`h` are the absolute
/// cell rect (not pixels) within the pane; `node` is the leaf drawn there
/// (`Grid`, `Buffer`, or `Graphics`).
#[derive(Debug, Clone)]
pub struct PositionedWindow {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// Game-pixel origin/size (font cell = 8 px) for the Phase 1c pixel
    /// composite. `x`/`y`/`w`/`h` above are the cell-quantized rect used by the
    /// Phase 1b fallback; these preserve the sub-cell offset it discards.
    pub x_px: u16,
    pub y_px: u16,
    pub w_px: u16,
    pub h_px: u16,
    /// Text left/right margins (pixels) set by the game via `set_margins` — the
    /// inset that keeps a window's text inside a graphical border frame. Applied
    /// when rasterizing this window's text into the pixel canvas (0 = flush to
    /// the window edge). v6 only; 0 elsewhere.
    pub left_margin: u16,
    pub right_margin: u16,
    pub node: WinNode,
}

/// A node in the engine-neutral window tree.
#[derive(Debug, Clone)]
pub enum WinNode {
    /// A split of two child windows.
    Pair {
        vertical: bool,
        split: Split,
        /// The split's `winmethod_Border` hint (true = a separator between the
        /// children); rendered in T4.
        border: bool,
        /// The KEY (new) window's Normal-style background colour (packed RGB),
        /// or `None` if unset — the colour the between-siblings separator adopts.
        key_bg: Option<u32>,
        /// The KEY window's Normal-style foreground colour (packed RGB), or `None`.
        key_fg: Option<u32>,
        first: Box<WinNode>,
        second: Box<WinNode>,
    },
    /// A text-grid window.
    Grid(GridWindow),
    /// A text-buffer window.
    Buffer(BufferWindow),
    /// A pixel-canvas graphics window.
    Graphics(GraphicsWindow),
    /// An empty placeholder.
    Blank,
    /// A v6 z-ordered layered composite (Phase 1b): an ordered list of windows
    /// placed at absolute cell rects within the story pane. Drawn in list
    /// order — earlier entries are background (graphics), later entries paint
    /// on top (text); a `Grid` leaf paints only its non-blank cells so an
    /// earlier layer shows through gaps ("cell-text-wins").
    Layered(Vec<PositionedWindow>),
}

/// The right-hand field of a classic (v3-style) status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusField {
    ScoreTurns { score: i16, turns: u16 },
    Time { hours: u8, minutes: u8 },
}

/// The status the app draws above the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusModel {
    /// A classic automatic status line (location + score/turns or clock); the
    /// app renders it through its configurable status-bar layout.
    Classic { location: String, right: StatusField },
    /// The engine has no automatic status line; the app shows its own
    /// (detected room + turn counter) info instead.
    HostManaged,
}

/// The whole screen as an engine-neutral window tree plus the status the app
/// draws as chrome.
#[derive(Debug, Clone)]
pub struct ScreenModel {
    /// The window tree.  In 3b-i this is the degenerate `Pair { Grid, Buffer }`.
    pub root: WinNode,
    /// The status line the app draws above the transcript.
    pub status: StatusModel,
    /// The game's current background colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to paint the story pane.
    pub bg: u32,
    /// The game's current foreground colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to colour the live input line.
    pub fg: u32,
    /// The extent (cols, rows) gvm's window tree actually covers; may be smaller
    /// than the story pane because gvm snaps proportional splits to whole cells and
    /// leaves a blank margin (SQ-0303). The generic multi-window composite clamps to
    /// this so the margin stays blank rather than stretching the last window. `(0, 0)`
    /// means unknown (the simple/Z-machine paths, which have no snap margin) → the
    /// composite falls back to the full pane.
    pub content_size: (u16, u16),
}

impl ScreenModel {
    /// Borrow the first [`GridWindow`] in the tree (the upper/status grid), if any.
    pub fn grid(&self) -> Option<&GridWindow> {
        fn find(node: &WinNode) -> Option<&GridWindow> {
            match node {
                WinNode::Grid(g) => Some(g),
                WinNode::Pair { first, second, .. } => find(first).or_else(|| find(second)),
                _ => None,
            }
        }
        find(&self.root)
    }
}

// ── Introspection capability ────────────────────────────────────────────────

/// Read-only introspection into the game world that drives the play-aids
/// (autocomplete vocabulary, inventory strip, room inspector, inventory
/// tracking).  An engine without introspection (e.g. an Inform-7 Glulx game
/// before symbol support exists) returns `None` from [`Engine::introspect`] and
/// the aids degrade gracefully.
///
/// Object handles are opaque `u16` identifiers; their meaning is engine-defined.
///
/// **Every object question answers with [`ObjectWords`]** — the thing's id, what
/// the story PRINTS for it, and the words the parser will ACCEPT for it, as one
/// value (SQ-1042). The three used to answer with display names alone, which
/// left every caller guessing which word of "a battery-powered brass lantern"
/// the parser had agreed to; Zork I accepts none of them and answers to `lamp`,
/// `lanter` and `light` instead. Handing back a name without its words is what
/// made the command band offer something the story never promised, and handing
/// back words without a name would leave a panel unable to say which thing they
/// belonged to — so neither half is offered on its own.
pub trait Introspect {
    /// The parser vocabulary (used to seed autocomplete at startup).
    fn vocabulary(&self) -> Vec<String>;
    /// The direct children of `container` (the inventory strip passes the
    /// player object here). One level, always — see
    /// [`Self::visible_contents`] for the question SCOPE asks.
    fn contents(&self, container: u16) -> Vec<ObjectWords>;
    /// The objects located directly in `room`.
    fn room_objects(&self, room: u16) -> Vec<ObjectWords>;
    /// Same as [`Self::room_objects`], but omitting `exclude` (the command
    /// band's "here" column passes the player object — SQ-0667). The player
    /// object is structurally a child of whatever room they're in, so
    /// without this it would show up in every room of every game; excluded
    /// by id, deliberately not by name (a scenery object could coincidentally
    /// share the player's printed name).
    fn room_objects_excluding(&self, room: u16, exclude: Option<u16>) -> Vec<ObjectWords>;
    /// The contents of `container` the player can SEE: its direct children,
    /// plus the contents of any child whose contents are visible, as deep as
    /// the engine's own containment model will vouch for (SQ-1133).
    ///
    /// [`Self::contents`] answers a different question — what is in your hands
    /// *right now*, which is the inventory dock's list and is one level by
    /// definition. This one is the carried half of SCOPE: `open sack` puts the
    /// lunch inside it within reach, and a panel that offers the word for it is
    /// offering something the parser will accept. A shut container never
    /// contributes; that is the engine's guarantee, not the caller's check.
    ///
    /// The default is the direct children, which is the whole truth for an
    /// engine with no notion of an open container.
    fn visible_contents(&self, container: u16) -> Vec<ObjectWords> {
        self.contents(container)
    }
    /// **Every object in the story**, with the words its parser files each one
    /// under — the story's whole vocabulary of THINGS, at any distance and in
    /// any state (SQ-1135).
    ///
    /// Not a scope question and not a spoiler on its own: the caller is the
    /// command band's printed-word block, which asks it only about words the
    /// story has ALREADY PRINTED, to tell a thing from a function word.
    ///
    /// It exists because the DICTIONARY cannot answer that question everywhere.
    /// The flag byte's noun bit is the obvious test and it is wrong on the three
    /// Infocom Version 6 games, whose layout `zvm::grammar::decode_roles` reads
    /// only `verb` from: measured on the churchyard frame, Arthur's noun bit
    /// picks out `are is was were will` and misses `crystal`, `torque` and
    /// `sword`; Zork Zero's picks `a all and of the then`; Shogun's picks
    /// nothing at all. An object's parse names are the story's own answer and
    /// need no flag layout.
    ///
    /// `None` means the question could not be ASKED — an engine with no such
    /// list, which is Scott Adams today. Glulx answers the folded-set form
    /// through [`Engine::object_word_set`] instead (`gvm::objects::ParseNames`,
    /// SQ-1210) without implementing this trait, whose tree questions it cannot
    /// answer — see that method for why the two capabilities are separate
    /// seams. An empty `Some` is a story that was asked and holds no parse
    /// names anywhere, which is what Journey and `advent.z8` really are. A
    /// caller that flattens the two reports "this story names no things" about
    /// one it never managed to read.
    fn all_object_words(&self) -> Option<Vec<ObjectWords>> {
        None
    }
    /// [`all_object_words`](Introspect::all_object_words) folded into the one
    /// question its bulk callers ask — **does ANY object answer to this word**
    /// — as a set with O(1) membership (SQ-1176).
    ///
    /// The reveal asks it for every token on screen and `refresh_seen_words`
    /// for every freshly printed word, each against every object's every word;
    /// walking `Vec<ObjectWords>` with `refers_to` re-truncates the whole
    /// vocabulary per token. The set truncates each stored word once. A caller
    /// that needs to know *which* object answers still wants the `Vec`.
    ///
    /// `None` means exactly what it means on `all_object_words`: the question
    /// could not be asked. An empty set is a story that was asked and holds no
    /// parse names. The `Arc` lets an engine hand out one cached build — the
    /// words are static story data in practice, though a game CAN rewrite them,
    /// so an implementation that caches must invalidate whenever the VM runs
    /// (see `GameSession::object_word_set`).
    fn object_word_set(&self) -> Option<std::sync::Arc<ObjectWordSet>> {
        self.all_object_words().map(|objs| std::sync::Arc::new(ObjectWordSet::build(&objs)))
    }
    /// The object handles whose parent is `parent` (drives inventory tracking).
    fn children_of(&self, parent: u16) -> std::collections::BTreeSet<u16>;
    /// The player object, if it can be identified.
    fn player_object(&self) -> Option<u16>;
}

// ── Debugger capability ──────────────────────────────────────────────────────

/// Static confidence provenance of a disassembled line (SQ-0428): where the
/// disassembler's classification of those bytes came from. Engine-neutral mirror
/// of the zvm cache's provenance; the render layer combines it with the runtime
/// executed-PC overlay (`executed_pcs`) to pick a final colour tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisasmProvenance {
    /// Hard: RD-discovered / initial-PC / execution-confirmed code.
    Rd,
    /// Soft: an unverified linear-scan guess.
    Soft,
    /// Not code: an opaque `.byte` run.
    Data,
}

impl From<zvm::cpu::disasm_cache::Provenance> for DisasmProvenance {
    fn from(p: zvm::cpu::disasm_cache::Provenance) -> Self {
        use zvm::cpu::disasm_cache::Provenance as P;
        match p {
            P::Rd => DisasmProvenance::Rd,
            P::Soft => DisasmProvenance::Soft,
            P::Data => DisasmProvenance::Data,
        }
    }
}

/// Read-only debug inspection of a running engine. All methods return
/// pre-formatted lines so the app render code stays engine-neutral (mirrors
/// `Engine::window_dump`). Z-machine implements this; other engines return
/// `None` from `Engine::debugger` for now. (Inspect-only; a stepper is a
/// future increment that will add `&mut` control methods.)
pub trait Debugger {
    /// Instruction pointer the VM is parked at (for "jump to PC").
    fn pc(&self) -> u32;
    /// Disassemble `lines` instructions starting at `addr`, one string per line.
    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Disassemble like [`disassemble`](Self::disassemble), but tag each line
    /// with its static confidence [`DisasmProvenance`] (SQ-0428). The default
    /// returns the `disassemble` text with every line tagged `Rd` — engines with
    /// no provenance model surface a single, uniform tier. The provenance is
    /// display-format-independent, so callers can pair it with the `basic`/`raw`
    /// text (whose lines match one-for-one).
    fn disassemble_tiered(&self, addr: u32, lines: usize) -> Vec<(String, DisasmProvenance)> {
        self.disassemble(addr, lines)
            .into_iter()
            .map(|s| (s, DisasmProvenance::Rd))
            .collect()
    }
    /// Raw disassembly: instruction bytes + decoded structure with NO lookups
    /// (no mnemonic name, operand-role sigils, variable naming, or packed-address
    /// unpacking) — a diagnostic view to catch bugs in the translation layer.
    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Basic disassembly: plain mnemonic form — named mnemonics, `#hex`/named-
    /// variable operands, and computed branch targets, but NO reference-following
    /// (no operand-role sigils, packed-address unpacking, `VarRef`, or annotations).
    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Address of the instruction after the one at `addr` (clamped to memory);
    /// lets the panel advance the disassembly view by whole instructions.
    fn next_instr(&self, addr: u32) -> u32;
    /// Start address of the instruction before `addr` (for backward scrolling).
    fn prev_instr(&self, addr: u32) -> u32;
    /// Human-readable help lines for the instruction at `addr` (what the opcode
    /// does, its operand roles, store/branch) — for the hover tooltip. Returns
    /// `None` if the engine has no descriptions or `addr` isn't an instruction.
    fn describe_line(&self, _addr: u32) -> Option<Vec<String>> {
        None
    }
    /// The set of instruction start-PCs executed during the last command turn
    /// (empty until a turn runs with tracing on).
    fn executed_pcs(&self) -> std::collections::HashSet<u32>;
    /// The cumulative set of instruction start-PCs ever executed while tracing
    /// was on (never cleared per turn), plus any host-seeded prior coverage.
    /// Drives the permanent "executed" disassembly colour. Default empty for
    /// engines/doubles with no cumulative model. (SQ-0449)
    fn ever_executed_pcs(&self) -> std::collections::HashSet<u32> {
        std::collections::HashSet::new()
    }
    /// Call stack, one or more lines per frame, innermost last.
    fn stack_lines(&self) -> Vec<String>;
    /// Evaluation/value stack, top first, marking frame-base boundaries.
    fn eval_stack_lines(&self) -> Vec<String>;
    /// Locals of the innermost frame.
    fn locals_lines(&self) -> Vec<String>;
    /// Global variables, formatted.
    fn globals_lines(&self) -> Vec<String>;
    /// The object tree, indented.
    fn object_tree_lines(&self) -> Vec<String>;
    /// Dictionary words.
    fn dictionary_lines(&self) -> Vec<String>;
    /// Hex+ASCII dump: `rows` rows of 16 bytes from `addr`.
    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String>;
    /// Total addressable memory length (so the panel can clamp scroll).
    fn memory_len(&self) -> u32;
    /// Detail lines for object `obj`: its set attributes, then its property
    /// table (number → hex bytes) — shown inline when the Objects tree entry
    /// is expanded.
    fn object_detail(&self, obj: u16) -> Vec<String>;
    /// Detail lines for call-stack frame `idx`: its locals (`localN = 0x…… (N)`),
    /// shown inline when the Call Stack frame entry is expanded.
    fn frame_locals(&self, idx: usize) -> Vec<String>;
    /// Current value of Z-machine variable `var` (0 = top of the eval stack,
    /// 1..=15 = locals of the innermost frame, 16..=255 = globals). `None` when
    /// unavailable (no frame, empty stack, or no such local). Read-only peek —
    /// never pops. Lets the Memory jump box dereference a variable to an address.
    fn var_value(&self, var: u8) -> Option<u16>;

    /// The story's own encoded text, laid out row-for-row against the window
    /// [`memory_hex`](Self::memory_hex) formats for the same `addr`/`rows`.
    ///
    /// Element *i* is the text that row *i*'s bytes produced; `None` means no
    /// string the story's tables account for covers that row, and the caller
    /// must fall back to the raw character column rather than guess.
    ///
    /// The Memory view's char column maps one byte to one ZSCII code, but a
    /// dictionary key and an object short name are Z-encoded — three 5-bit
    /// Z-characters packed per 16-bit word (ZMSD §3.2) — so that column shows
    /// noise over exactly the entries the Objects/Dictionary tabs let you jump
    /// to (SQ-0448/SQ-0969). Nothing can be decoded per byte, and nothing can be
    /// decoded from an arbitrary row boundary either: the decoder carries an
    /// alphabet shift and a pending abbreviation across words, so a decode
    /// started mid-string is wrong rather than offset, and looks plausible. So
    /// an implementation must anchor every row it fills in to a string START
    /// address it actually knows, and leave the rest `None`.
    ///
    /// A short (or empty) vec is fine and means "no text for the rows past its
    /// end" — the default is empty, which is what engines with no Z-text
    /// (Glulx, Scott) want and costs the render site no special-casing.
    fn memory_zstrings(&self, _addr: u32, _rows: usize) -> Vec<Option<String>> {
        Vec::new()
    }

    /// This engine's per-window inspector tab layout, or `None` to use the
    /// panel's default [`WINDOW_TABS`](crate::debug_panel::WINDOW_TABS). Each
    /// inner slice lists one window's tabs, in order; any [`Section`] not listed
    /// is hidden. Lets an engine with no call stack / eval stack / linear memory
    /// (e.g. Scott Adams) drop those tabs and reuse the plain-list slots for its
    /// own content. Default `None` keeps the Z-machine panel byte-for-byte.
    fn sections(&self) -> Option<[&'static [crate::debug_panel::Section]; 3]> {
        None
    }

    /// The tab label to show for `s` under this engine. Default is the section's
    /// own [`label`](crate::debug_panel::Section::label); an engine that reuses a
    /// section slot for different content (Scott shows "Items" on the Objects
    /// tab, "World" on the Locals tab) overrides this to relabel it.
    fn section_label(&self, s: crate::debug_panel::Section) -> &'static str {
        s.label()
    }
}

// ── Engine-tagged save ──────────────────────────────────────────────────────

/// The location/room currency shared between the engine and the mapper.
pub type LocationInfo = zvm::ObjectSnapshot;

/// A persisted game state, tagged with the engine that produced it.
///
/// The archive records [`engine`](Self::engine) so a restore can refuse a save
/// written by a different engine.  `bytes` is the engine-defined save blob
/// (Quetzal for `zvm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSave {
    /// The engine tag (e.g. `"zmachine"`).
    pub engine: String,
    /// The save format version within that engine.
    pub format_version: u32,
    /// The engine-defined save bytes.
    pub bytes: Vec<u8>,
}

impl EngineSave {
    /// Build a save tagged for `engine`.
    pub fn new(engine: impl Into<String>, format_version: u32, bytes: Vec<u8>) -> Self {
        EngineSave { engine: engine.into(), format_version, bytes }
    }

    /// True when this save was produced by `engine`.
    pub fn is_engine(&self, engine: &str) -> bool {
        self.engine == engine
    }
}

/// One host snapshot per finished turn, taken lazily and shared by everything
/// that wants the post-turn state (SQ-1178).
///
/// With history and auto-save on, one turn used to pay [`Engine::save_state`]
/// three times over — the history capture, the per-turn archive write, and the
/// return probe's snapshot — for one identical moment: nothing between the
/// turn being applied and the next command mutates the VM (the word
/// refreshers, inventory tracking and world prints all read through
/// `&dyn Engine`). At ~102 ms per call on Counterfeit Monkey in a debug
/// build, that was the biggest per-turn cost on Glulx.
///
/// Lazy, not eager, because every consumer is gated — history and auto-save by
/// config, the return probe by a crossing the map has no way back from — and a
/// turn none of them fires on must keep costing nothing. The first
/// [`get`](Self::get) pays; the rest share the same blob.
///
/// One value lives per finished turn, as a local in the turn finisher — never
/// across turns, because the next command invalidates it.
#[derive(Default)]
pub struct TurnSave(Option<std::sync::Arc<EngineSave>>);

impl TurnSave {
    /// This turn's snapshot, taking it on first use.
    pub fn get(&mut self, session: &dyn Engine) -> std::sync::Arc<EngineSave> {
        std::sync::Arc::clone(
            self.0.get_or_insert_with(|| std::sync::Arc::new(session.save_state())),
        )
    }
}

/// An engine operation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// A save written by a different engine was offered for restore.
    EngineMismatch { expected: String, found: String },
    /// The save bytes were rejected by the engine (corrupt / wrong story).
    BadSave(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::EngineMismatch { expected, found } => write!(
                f,
                "save engine mismatch: expected {expected}, found {found}"
            ),
            EngineError::BadSave(msg) => write!(f, "bad save: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

// ── The Engine trait ────────────────────────────────────────────────────────

/// The app-facing handle to a running game, independent of the underlying VM.
///
/// The app holds a `Box<dyn Engine>` where it once held a concrete
/// `GameSession`.  The `zvm` adapter lives in `session.rs`.
pub trait Engine {
    // ── turn cycle ──
    /// Supply a player command and run to the next input request / quit.
    fn submit(&mut self, command: &str) -> TurnResult;
    /// Supply a single keypress.  Returns `None` when the key has no input
    /// meaning for this engine (e.g. an arrow key under the Z-machine), in
    /// which case the caller leaves the turn untouched.
    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult>;
    /// Drain the transcript accumulated since the last drain.
    fn take_transcript(&mut self) -> String;
    /// Drain the accumulated transcript as ordered elements (text runs + inline
    /// images) — the element counterpart to `take_transcript`, mirroring the
    /// per-turn `TurnResult::transcript_elems`. The DEFAULT returns empty, meaning
    /// "no ordered elements; use the flat `take_transcript` string path" — the
    /// Z-machine has no inline images, so it keeps the default and drains nothing
    /// here. `GlulxSession` overrides it so banner/startup images survive.
    fn take_transcript_elems(&mut self) -> Vec<crate::session::TranscriptElem> {
        Vec::new()
    }
    /// Did the last KEYPRESS turn's output start printing exactly where the
    /// previous output left the game's cursor — i.e. does it continue that line
    /// rather than open a new one (SQ-0804)?
    ///
    /// The host puts every turn's output on a fresh transcript line, which is
    /// how it supplies the newline an interpreter echoes after a `read` (ZMSD
    /// §7.1.1.1). `read_char` echoes nothing at all (§10.7), so for a keypress
    /// turn that newline is the host's own invention, and whether it belongs
    /// cannot be read off the printed text: a game redrawing a menu `set_cursor`s
    /// back to the top and prints no newline either way. The game's cursor
    /// answers it exactly, so this is where the answer comes from.
    ///
    /// DEFAULT `false` — "assume a new line", which is what every engine did
    /// before and what any engine without a screen model must keep doing. Only
    /// the Z-machine's v6 path can answer, because only it models a cursor;
    /// v1–v5 stories, Glulx and Scott all keep the default.
    fn output_continued_line(&self) -> bool {
        false
    }
    /// When false, the game's own trailing `>` read prompt is preserved in the
    /// transcript (inline-prompt mode) instead of being stripped for the app's
    /// dedicated input bar. Default true.
    fn set_strip_prompt(&mut self, _on: bool) {}
    /// Which kind of input the VM is currently waiting for.
    fn pending_input(&self) -> InputKind;
    /// Resume after the host performed an in-game SAVE.
    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult;
    /// Resume after the host performed an in-game RESTORE.
    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult;
    /// The pending `create_by_prompt` filename request, if the VM suspended on one
    /// this turn. Default `None` (only the Glulx engine issues these).
    fn pending_filename(&self) -> Option<FilenameReq> {
        None
    }
    /// The user-visible VFS filenames, for a `create_by_prompt` read picker.
    /// Default empty (engines without a Glk VFS).
    fn file_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Resume after the host chose a filename (or cancelled with `None`) for a
    /// `create_by_prompt`. Only valid for engines that produce filename requests;
    /// the default panics because the run loop only calls this when
    /// [`Engine::pending_filename`] returned `Some`.
    fn resume_filename(&mut self, _name: Option<String>) -> TurnResult {
        unreachable!("resume_filename is only valid for engines that issue filename requests (Glulx)")
    }
    /// Whether the game has ended.
    fn has_quit(&self) -> bool;

    // ── screen ──
    /// The current screen as a neutral window tree + status — the SETTLED state,
    /// i.e. everything the game has drawn so far.
    fn screen(&self) -> ScreenModel;

    /// The screen as the player is seeing it THIS INSTANT, which is the settled
    /// [`screen`](Engine::screen) for every engine but one.
    ///
    /// The exception is a v6 Z-machine turn that queued several `draw_picture`s
    /// (SQ-0708): the renderer plays those out over successive frames instead of
    /// handing the player the finished composite at once, so mid-sequence this
    /// answers with the frame that is up rather than the one the turn ended on.
    /// The sequence always ends on `screen()`'s composite, byte for byte — which
    /// is why everything that wants the game's state (saves, the display list,
    /// `/dump-windows`) keeps asking `screen()` and is unaffected by pacing.
    ///
    /// Handed back as a SHARED `Arc` (SQ-1191): the zvm session memoizes the
    /// built model and returns the same allocation for every frame on which
    /// nothing changed, so a caller must treat it as read-only — which the
    /// render has done since its `ChromeStrip` runs became owned (SQ-1187).
    fn screen_now(&self) -> std::sync::Arc<ScreenModel> {
        std::sync::Arc::new(self.screen())
    }

    /// A diagnostic dump of the live window layout, one line per entry, for the
    /// `/dump-windows` command. The default gives a one-line Z-machine summary
    /// (the grid dims + the buffer); engines with a real Glk window tree (Glulx)
    /// override this to print the full indented tree with per-window colours.
    fn window_dump(&self) -> Vec<String> {
        let model = self.screen();
        let (gc, gr) = model.grid().map(|g| (g.cols, g.active_rows)).unwrap_or((0, 0));
        vec![format!("Window layout: Grid {}x{} over Buffer (Z-machine simple path)", gc, gr)]
    }

    /// Enable/disable the `screen` trace on this engine's VM (default: no-op for
    /// engines without a Glk/screen model, e.g. Scott). (trace feature)
    fn set_trace_screen(&mut self, _on: bool) {}

    /// Drain any accumulated `screen`-trace lines (display instructions the story
    /// issued this turn). Default empty; zvm/gvm sessions override. (trace feature)
    fn take_screen_trace(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Build a per-turn `v6` debug-trace snapshot (window geometry, paint runs,
    /// picture canvases) — read directly from live state, not drained from a
    /// buffer. `None` when the engine has no v6 model at all (Glulx, Scott) OR
    /// the loaded story isn't v6; `GameSession` overrides this. Default `None`.
    /// (trace feature)
    fn v6_snapshot(&self) -> Option<Vec<String>> {
        None
    }

    /// Report a mouse click at game-pixel `(y_px, x_px)` (1-based) to the engine,
    /// so a subsequent `read_mouse` (or the game reading the header extension
    /// table) sees the click coordinates. Default no-op: only the v6 Z-machine
    /// consumes host mouse coordinates this way (Glulx has its own Glk mouse
    /// event path). (Lane M)
    fn set_mouse(&mut self, _y_px: u16, _x_px: u16) {}

    /// The v6 screen's PAINTED ground — filled rectangles left by `erase_window`,
    /// in native pixels (SQ-0706). `None` for every engine and every game that
    /// never paints one, which is all of them but scopa-shaped v6 titles.
    ///
    /// Deliberately outside [`Engine::screen`]: it is a pixel surface, not a
    /// window tree, and the renderer composites it as a BACKDROP beneath the
    /// chrome and the story text rather than as another window.
    fn paint_surface(&self) -> Option<std::sync::Arc<image::RgbaImage>> {
        None
    }

    /// Report the host's REAL screen size, in character cells, to the story.
    ///
    /// ZMSD §8.4: the interpreter "may change the exact dimensions whenever it
    /// likes but must write the current height (in lines) and width (in
    /// characters) into bytes $20 and $21 in the header" (v5+ also mirrors them
    /// into the unit words at $22/$24, §8.4.3). The host therefore calls this at
    /// first layout and on every terminal resize with the story pane's measured
    /// size — see `loop_tick::poll_zvm_resize`.
    ///
    /// Default no-op: Glulx re-arranges through its own `GlulxSession::resize`
    /// (Glk windows, not a header), and Scott has no screen model. `GameSession`
    /// overrides it. (SQ-0532/A-F1)
    fn set_screen_dims(&mut self, _rows: u16, _cols: u16) {}

    /// Publish the interpreter's OWN default background/foreground colours, as
    /// ZMSD §8.3.1 standard colour numbers, into header bytes $2C/$2D.
    ///
    /// §8.3.3: "If the interpreter can produce colours, it should set bit 0 of
    /// 'Flags 1' in the header, and write its default background and foreground
    /// colours into bytes $2c and $2d of the header." The host resolves the pair
    /// from the active theme / the OSC-probed terminal (see
    /// [`crate::colors::host_default_colour_pair`]) so the bytes describe what is
    /// actually on screen. Default no-op — non-Z engines have no such header.
    /// (SQ-0532/A-F2)
    fn set_default_colours(&mut self, _bg: u8, _fg: u8) {}

    /// Enable/disable per-turn execution tracing for the debug inspector.
    fn set_debug_trace(&mut self, _on: bool) {}

    /// Seed the cumulative "ever executed" coverage set from host-persisted
    /// knowledge (the debug PC-set sidecar) so prior runs' coverage colours the
    /// disassembly immediately. Default no-op for engines with no such model.
    /// (SQ-0449)
    fn seed_executed_pcs(&mut self, _pcs: &std::collections::HashSet<u32>) {}

    // ── persistence (engine-tagged) ──
    /// Capture the game state as an engine-tagged save.
    fn save_state(&self) -> EngineSave;
    /// Restore from an engine-tagged save.  Refuses a foreign-engine save.
    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError>;
    /// Restore the bytes an in-game `@save` seals — a bare standard Quetzal /
    /// Glulx-Quetzal blob — through the HOST path (the saves manager,
    /// `/restore-state`, an interchange file carried in), by completing the save
    /// instruction's descriptor and running on to the next input prompt.
    ///
    /// Every engine implements this, and that is deliberate (SQ-0556): uniform
    /// in-game-save semantics mean an `@save` archive is loadable from the saves
    /// manager on ANY engine, not just through the game's own `@restore`. A new
    /// engine is not done until its `@save` archive behaves like the others.
    /// What "completing the descriptor" means is per-engine (the Z-machine
    /// branches true / stores 2; Glulx pops `@save`'s call stub and stores the
    /// `-1` sentinel; Scott has no descriptor and simply loads its snapshot).
    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError>;
    /// Whether a game-initiated `@save`/`@restore` is currently suspended,
    /// awaiting host file I/O. Hosts must skip any unconditional host-snapshot
    /// trigger (e.g. exit auto-save) while this is true — snapshotting mid-
    /// suspension would capture an un-popped Glulx `@save` call stub, corrupting
    /// the stack on a later Save State restore. Default `false` (only Glulx's
    /// stub-based `@save` has this hazard; the Z-machine's descriptor-based
    /// `@save` does not).
    fn is_saveload_pending(&self) -> bool {
        false
    }

    // ── auxiliary persistent data (neutral byte map) ──
    /// The engine's auxiliary persistent data table.
    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>>;
    /// Replace the auxiliary persistent data table.
    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>);
    /// Whether the auxiliary data changed since the last clear.
    fn aux_dirty(&self) -> bool;
    /// Clear the auxiliary-data dirty flag.
    fn clear_aux_dirty(&mut self);

    // ── Glk file VFS (Glulx only; default no-ops for the Z-machine) ──
    /// Encode the Glk file VFS as a disk sidecar blob (empty for engines
    /// without a Glk VFS).
    fn vfs_bytes(&self) -> Vec<u8> { Vec::new() }
    /// Replace the Glk file VFS from a disk sidecar blob (no-op if unsupported).
    fn load_vfs(&mut self, _bytes: &[u8]) {}
    /// Whether the Glk file VFS changed since the last clear.
    fn vfs_dirty(&self) -> bool { false }
    /// Clear the Glk file VFS dirty flag.
    fn clear_vfs_dirty(&mut self) {}

    // ── mapping ──
    /// The player's current location, for the mapper.
    fn current_location(&self) -> Option<LocationInfo>;

    /// What `origin`'s own map data declares for `dir` (SQ-1257) — read from
    /// the story's compiled exit table, never from anything ever walked.
    ///
    /// This is what lets the mapper tell a REAL passage from one a routine
    /// improvised on the spot: Lost Pig's gnome tunnels relocate the player
    /// somewhere the room's own exit table never named, and a caller that
    /// compares this against where the player actually landed is the whole of
    /// what tells the two apart. `RoomId` is `mapper::graph::RoomId`, which for
    /// every engine here is that engine's own object-number space.
    ///
    /// Default `DeclaredExit::Unknown`: an engine with no such table (Scott
    /// Adams, or a Glulx story — Inform 7 for Glulx keeps its map in Glulx
    /// memory in a shape this seam does not read yet) has nothing to answer
    /// with, which is a real answer and not a failure to look. `GameSession`
    /// (Z-machine) is the only override today.
    fn declared_exit(
        &self,
        _origin: mapper::graph::RoomId,
        _dir: mapper::direction::Direction,
    ) -> DeclaredExit {
        DeclaredExit::Unknown
    }

    /// This engine's current random-number seed, when it exposes one (SQ-1257
    /// Phase 2) — `zvm`'s `random` opcode xorshift32 state. `None` for an
    /// engine with no seed to read (Glulx, Scott) or none of `declared_exit`'s
    /// `Absent`/`Code` answers are ever worth a reseeded probe for anyway.
    fn rng_seed(&self) -> Option<u32> {
        None
    }

    /// Force this engine's random-number generator to `seed` (SQ-1257 Phase
    /// 2): the shadow's own draw, made to differ from the live game's, so a
    /// probe walking the same command twice under two different seeds can
    /// tell "the story rolled dice" apart from "the story is deterministic
    /// and my snapshot happened to agree with itself twice". Default no-op —
    /// an engine that answers `None` from [`Self::rng_seed`] has nothing here
    /// worth forcing either.
    fn reseed_random(&mut self, _seed: u32) {}

    /// Opaque, engine-defined bytes describing whatever HOST-SIDE state this
    /// engine's room ids currently depend on — carried from the live session
    /// into a [`crate::probe`] shadow so the shadow keys rooms exactly as the
    /// live session does (SQ-1267).
    ///
    /// A Glulx `RoomId` is a hash of either the room object's ADDRESS (once
    /// the story's `location` global has been located — see
    /// `glulx_roomlock`) or, before that, of the room's printed NAME — and
    /// which of the two is in force is host-side bookkeeping a [`EngineSave`]
    /// snapshot never carries (a gvm snapshot is VM memory only). A shadow
    /// left to learn this on its own, from its own exploratory commands, can
    /// answer with a THIRD id that matches neither: the live session's own
    /// address-derived hash if it has locked, or a stale/absent guess if it
    /// has not. Default `None`: an engine whose room ids need no such
    /// state — `GameSession`'s are `zvm`'s own object numbers, fixed by the
    /// story compile and identical in the live session and any shadow of it.
    fn room_identity_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// Apply a [`Self::room_identity_state`] captured from the live session.
    /// Called by the probe worker immediately after every `restore_state`,
    /// before the shadow runs a command, so a shadow reused across many
    /// questions is re-synced to the live session's CURRENT identity state on
    /// every one of them rather than only at boot. Default no-op.
    fn apply_room_identity_state(&mut self, _state: &[u8]) {}

    // ── boot ──
    /// Drain the game's pending screen clear — the fact [`TurnResult::erase_lower`]
    /// carries, taken on its own.
    ///
    /// Every engine already drains this inside its own per-turn path; this is that
    /// same drain, reachable by the BOOT, which is not a turn and does not go
    /// through it. Deliberately **required** rather than defaulted: an engine that
    /// answers `false` is stating it has no screen-clear channel (Scott), not
    /// forgetting that it has one.
    fn drain_screen_clear(&mut self) -> bool;

    /// The boot's own turn: everything the game did between reset and its first
    /// input request, as the [`TurnResult`] the host seeds the session with.
    ///
    /// The boot IS a turn — the player just never typed it — so it must drain the
    /// same per-turn channels a real turn drains. `startup.rs` used to hand-build
    /// this as a `TurnResult { … }` literal with `erase_lower: false` spelled into
    /// it, which left an `erase_window` issued during boot sitting on the engine
    /// until the FIRST REAL TURN's drain took it: the banner and the opening room
    /// description were wiped one command late, on every v5 Infocom re-release that
    /// clears the screen before printing its banner (SQ-1106 — the same omission
    /// shape as SQ-0901 / SQ-1020 / SQ-1022, and the reason this is a constructor
    /// rather than a literal).
    ///
    /// The drained clear is CARRIED, not acted on: the host seeds the map from this
    /// result and nothing on the boot path marks a screen-clear boundary. That is
    /// correct, and is what `zvm-cli` does with the same erase — the screen the game
    /// cleared is the one before its banner, and the host has drawn nothing yet, so
    /// clearing it is invisible. Marking a boundary here would anchor the screen
    /// BELOW the banner the boot printed *after* the erase, i.e. hide it, which is
    /// the reported bug wearing the other hat.
    ///
    /// **It drains the clear and nothing else, and that is a measured answer rather
    /// than the next omission** (SQ-1109, which handed itself over from SQ-1106 on
    /// the reasoning that the three OTHER per-turn facts a `TurnResult` carries must
    /// leak the same way). They were looked for and are not there:
    ///
    /// - `pictures` already has a boot drain, just not in here —
    ///   [`crate::session::GameSession::flush_boot_pictures`], called from
    ///   `startup.rs`, `reset.rs` and `turn.rs` before this seed ever runs, because a
    ///   v6 game's opening art has to be on the canvas for the FIRST `screen()`, which
    ///   is drawn before the player types anything.
    /// - `sounds` and `diagnostics` have none, so a `sound_effect` or a VM diagnostic
    ///   raised during the game's own boot does sit on the machine until turn 1's
    ///   drain takes it. **Nothing in the corpus raises either.** Measured by booting
    ///   every story in `stories/` through `hints::load_mounted_story_from` — 277
    ///   Z-machine boots covering every loose file, Blorb and named entry of every
    ///   disk image, plus 26 Glulx boots — and reading `Machine::pending_sounds` /
    ///   `diagnostics` straight after the constructor: zero of each, everywhere. The
    ///   44 v6 boots left 3–220 pending pictures apiece, which is the drain above
    ///   doing its job.
    ///
    /// The route is nonetheless open, and can be forced: set Flags 2 bit 0 in
    /// `advent.z8`'s header before booting and `check_transcript_bit` raises its
    /// `output_stream 2` diagnostic at the BOOT's own input request — `seed_turn`
    /// returns no diagnostics and the next turn delivers it. Draining it here would
    /// not fix that, it would DISCARD it: the host feeds this result to the mapper
    /// and neither plays its sounds nor prints its diagnostics. Surfacing a boot
    /// sound and a boot warning at the boot is the real change, and it waits for a
    /// story that needs one.
    fn seed_turn(&mut self) -> TurnResult {
        TurnResult {
            location: self.current_location(),
            quit: self.has_quit(),
            erase_lower: self.drain_screen_clear(),
            ..TurnResult::default()
        }
    }

    // ── the story's own vocabulary (SQ-1041) ──

    /// What the story's parser accepts — its verbs and their sentence shapes,
    /// its whole dictionary, and how much of a word that dictionary keeps.
    ///
    /// Read ONCE a session and cached (`crate::vocab::VocabState`): the tables
    /// are static, so nothing a turn does can change an answer. `None` — the
    /// default — for an engine with nothing to give and for a story with no
    /// readable grammar, a menu-driven Version 6 game being the real example,
    /// and it means the vocabulary offer stays silent for the whole session.
    fn story_vocabulary(&self) -> Option<crate::vocab::StoryVocabulary> {
        None
    }

    /// Does the story's own dictionary hold this word — asked of the ENGINE, so
    /// the story's key truncation is applied the way the story applies it?
    ///
    /// `None` — the default — means "I have no lookup of my own; use the
    /// snapshot's". That is the right answer for Glulx and for a Scott Adams
    /// database, both of which truncate by plain characters, so
    /// [`crate::vocab::StoryVocabulary::knows`] is exact for them. The Z-machine
    /// truncates by Z-CHARACTERS — `flashlight` is stored as `flashl`, and a
    /// character outside alphabet A0 costs more than one of them — so only its
    /// own encoder can answer, and `GameSession` overrides this.
    fn knows_word(&self, _word: &str) -> Option<bool> {
        None
    }

    /// Split prose the way this story's own parser splits an input line
    /// (SQ-1116).
    ///
    /// The story printed the text; the story decides where one word ends. On the
    /// Z-machine this is not an approximation of the parser but the *identical
    /// code path* — `zvm::dictionary::tokenise`, the routine `read` itself calls
    /// — so the dictionary's declared separator characters (§13.1) are honoured,
    /// including a story that declares `-` or `'` and one that pointedly does not.
    ///
    /// `None` — the default — means "I have no tokeniser to lend"; the caller
    /// falls back to [`crate::complete::split_prose`]. That is where Glulx and
    /// Scott Adams sit today, and it costs them only an unusual separator set,
    /// because whatever comes out is still filtered through the story's own
    /// dictionary.
    fn split_like_parser(&self, _text: &str) -> Option<Vec<String>> {
        None
    }

    // ── capabilities / escape hatch ──
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Introspection capability, when the engine has one.
    fn introspect(&self) -> Option<&dyn Introspect> {
        None
    }
    /// **Does ANY object answer to this word** — the folded set of every
    /// object's parse names ([`Introspect::object_word_set`]), reachable
    /// without the rest of introspection (SQ-1210).
    ///
    /// It sits on `Engine` and not only on [`Introspect`] because the two are
    /// different capabilities that happened to travel together until Glulx
    /// could answer one and not the other. [`Introspect`]'s tree questions —
    /// contents, room objects, children — need object handles the app can
    /// correlate with rooms, which Glulx has none of (its objects are heap
    /// addresses, its rooms synthetic heading ids). Answering those with empty
    /// lists to smuggle the word set through `introspect()` would turn every
    /// "could not ask" into a false "asked, nothing there": `probe::WorldPrint`
    /// would fingerprint an empty world as a real one, the command band would
    /// label a column `here` off a tree that was never walked, and
    /// `vocab::scope_split` would stop saying `None`. So the word set gets its
    /// own seam and the tree questions keep refusing honestly.
    ///
    /// The default forwards through [`Self::introspect`], so an engine with
    /// full introspection (the Z-machine) answers here for free, with its own
    /// caching. The Glulx adapter overrides it (`gvm::objects::ParseNames`);
    /// Scott Adams keeps the `None`, which callers treat exactly as
    /// [`Introspect::object_word_set`] documents — the question could not be
    /// asked, distinct from an empty set.
    fn object_word_set(&self) -> Option<std::sync::Arc<ObjectWordSet>> {
        self.introspect().and_then(|i| i.object_word_set())
    }
    /// Debug-inspection capability, when the engine has one.
    fn debugger(&self) -> Option<&dyn Debugger> {
        None
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn key_event_to_input_maps_named_keys() {
        assert_eq!(key_event_to_input(key(KeyCode::Enter)), Some(KeyInput::Enter));
        assert_eq!(key_event_to_input(key(KeyCode::Backspace)), Some(KeyInput::Backspace));
        assert_eq!(key_event_to_input(key(KeyCode::Esc)), Some(KeyInput::Escape));
        assert_eq!(key_event_to_input(key(KeyCode::Up)), Some(KeyInput::Up));
        assert_eq!(key_event_to_input(key(KeyCode::Char('y'))), Some(KeyInput::Char('y')));
        assert_eq!(key_event_to_input(key(KeyCode::F(3))), Some(KeyInput::Func(3)));
        // A key with no neutral form maps to None.
        assert_eq!(key_event_to_input(key(KeyCode::CapsLock)), None);
    }

    #[test]
    fn screen_model_builds_and_finds_grid() {
        let mut grid = GridWindow::default();
        grid.resize(1, 5);
        grid.put(1, 1, 'H', 0);
        grid.put(1, 2, 'I', 2); // bold
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let g = model.grid().expect("tree has a grid");
        assert_eq!(g.cell(1, 1).ch, 'H');
        assert_eq!(g.cell(1, 2).ch, 'I');
        assert_eq!(g.cell(1, 2).style, 2);
        // Out-of-bounds is a blank default.
        assert_eq!(g.cell(9, 9).ch, ' ');
    }

    #[test]
    fn engine_save_round_trips_its_tag() {
        let save = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        assert_eq!(save.engine, "zmachine");
        assert_eq!(save.format_version, 1);
        assert_eq!(save.bytes, vec![1, 2, 3]);
        assert!(save.is_engine("zmachine"));
        assert!(!save.is_engine("glulx"));
    }

    #[test]
    fn engine_mismatch_error_displays() {
        let e = EngineError::EngineMismatch {
            expected: "zmachine".into(),
            found: "glulx".into(),
        };
        assert!(e.to_string().contains("zmachine"));
        assert!(e.to_string().contains("glulx"));
    }
}

#[cfg(all(test, feature = "t-session"))]
mod debugger_trait_tests {
    use super::*;

    struct Dummy;
    impl Debugger for Dummy {
        fn pc(&self) -> u32 { 0x4a2f }
        fn disassemble(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f  add".into()] }
        fn disassemble_raw(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f: 54 05 03 05   2OP:0x14".into()] }
        fn disassemble_basic(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f  loadw #0abc".into()] }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn prev_instr(&self, a: u32) -> u32 { a.saturating_sub(4) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn eval_stack_lines(&self) -> Vec<String> { vec!["(empty)".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { vec!["g00=0000".into()] }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        fn memory_hex(&self, _a: u32, _r: usize) -> Vec<String> { vec!["000000  00".into()] }
        fn memory_len(&self) -> u32 { 0x10000 }
        fn object_detail(&self, _obj: u16) -> Vec<String> { vec!["attrs: (none)".into()] }
        fn frame_locals(&self, _idx: usize) -> Vec<String> { vec!["local0 = 0x0001  (1)".into()] }
        fn var_value(&self, _var: u8) -> Option<u16> { None }
    }

    #[test]
    fn debugger_object_is_usable() {
        let d = Dummy;
        let dyn_d: &dyn Debugger = &d;
        assert_eq!(dyn_d.pc(), 0x4a2f);
        assert_eq!(dyn_d.next_instr(0x4a2f), 0x4a33);
        assert!(!dyn_d.disassemble(0, 4).is_empty());
    }
}
