//! Single-file archive bundling a story's map + VM save into one `.lanthorn` ZIP.
//!
//! # Integration points (for the follow-up wiring task)
//!
//! In `main.rs` / `session.rs`, replace the two separate persist calls:
//!
//!   ```text
//!   // save path: replace save_map + save_game with:
//!   archive::save_archive(&archive_path, &mapper, &machine)?;
//!
//!   // load path: replace load_map + restore_game with:
//!   let ac = archive::load_archive(&archive_path)?;
//!   let mapper = ac.mapper;
//!   machine.restore_quetzal(&ac.save).map_err(|e| ...)?;
//!   ```
//!
//! Archive path convention (mirrors `ifid::map_path`): `<base_dir>/<ifid>.lanthorn`
//!
//! The `meta.ifid` field is currently populated by the caller; pass `None` until
//! IFID computation is wired in. `load_archive` rejects only archives whose
//! `format_version` is GREATER than `CURRENT_FORMAT_VERSION`; older versions load
//! (history is read only when a `history/` index is present), so v1 archives load
//! with empty history.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;

use mapper::mapper::Mapper;
use mapper::persist::{from_json, to_json};

use crate::engine::EngineSave;

// ZIP entry names.
const ENTRY_MAP: &str = "map.json";
const ENTRY_META: &str = "meta.json";
const ENTRY_TRANSCRIPT: &str = "transcript.json";
const ENTRY_COMMAND_HISTORY: &str = "command_history.json";
const ENTRY_SCREEN: &str = "screen.json";
const ENTRY_DISPLAY: &str = "display.json";

/// The archived v6 screen as a RECIPE rather than a picture (SQ-0588): every
/// window's display list, plus the Current Palette they were drawn under.
///
/// Blorb §11.3 makes the palette part of the recipe. An adaptive picture carries
/// no palette of its own — it decodes through whichever one the last non-adaptive
/// draw established — so a display list replayed without the palette that was live
/// when it was recorded produces the right shapes in the wrong colours.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplayListDto {
    /// Raw `PLTE` bytes of the Current Palette at save time; `None` when no
    /// palette was ever established (a game with no indexed pictures).
    #[serde(default)]
    pub palette: Option<Vec<u8>>,
    /// One entry per REPLAYABLE window, in paint order (ascending `z_seq`) — the
    /// same order `pictures_png` emits, so relative z-order survives without
    /// storing the raw stamps. A window missing from here is one whose replay did
    /// not reproduce its canvas at save time; it is carried as a PNG instead.
    pub windows: Vec<V6WindowOpsDto>,
    /// The two v6 screen layers that ride BESIDE the window canvases (SQ-0814):
    /// the `erase_window` fills and the canvas anchors. Both are bounded, so both
    /// travel as a recipe rather than as pixels — unlike the painted ground, whose
    /// inputs are unbounded (`pictures/ground.png`).
    #[serde(default)]
    pub layers: V6LayersDto,
}

/// The v6 screen layers that live outside the window tree and outside the canvas
/// (SQ-0814): what the last `erase_window` on each window filled, and where each
/// window's canvas was painted.
///
/// Both were per-session before this — neither archived nor reset — so a host Save
/// State left the PRE-RESTORE screen's fills and anchors standing under the restored
/// window table. Nothing repaints them away: Quetzal saves no screen state by design
/// because the standard assumes the STORY repaints, and a Save State swaps memory
/// under a game that never learns it happened.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct V6LayersDto {
    /// The `erase_window` fills that are STILL COVERING, in PAINT ORDER (ascending
    /// draw stamp) — the order is the recipe, the stamp itself is not. Draw stamps
    /// come from a process-global counter, so a restore re-stamps these from the
    /// live counter exactly as the canvases are re-stamped, and only their relative
    /// order has to survive.
    ///
    /// Covering is the whole of a fill's remaining state. `session::WindowFill` also
    /// carries the window-0 character count at the moment it was painted, and
    /// `GameSession::screen` covers with a fill only while that count still EQUALS
    /// the live one — one character of prose since, and the fill is no longer the
    /// newest thing on the screen. That count only ever grows (its one reset,
    /// `@restart`, clears every fill in the same breath), so a fill the story has
    /// printed past can never cover again and carrying it would restore a record
    /// nothing can read. What travels is what still paints.
    pub fills: Vec<V6FillDto>,
    /// Where each window's canvas content was painted, ascending by window.
    pub anchors: Vec<V6AnchorDto>,
}

/// One window's surviving `erase_window` fill (`session::WindowFill`, SQ-0584), in
/// the game's own native pixels and a packed RGB colour — no cell geometry, so it
/// restores onto any terminal and any graphics backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct V6FillDto {
    pub win: u8,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Packed RGB (`0x00RRGGBB`) the erase painted; 0 is the page default.
    pub bg: u32,
}

/// Where one window's canvas was painted (`session::CanvasAnchor`, SQ-0715): the
/// window's 1-based screen origin at draw time, and the footprint the draws covered
/// in canvas coords. Native pixels throughout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct V6AnchorDto {
    pub win: u8,
    pub origin_x: u16,
    pub origin_y: u16,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// One v6 window's display list, with the canvas size to replay it into.
///
/// The size is stored rather than re-derived from the restored window table: the
/// canvas is created at the window's pixel box when the FIRST op lands, and a game
/// that resizes the window afterwards would otherwise replay into a canvas of the
/// wrong shape, silently clipping or padding the art.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct V6WindowOpsDto {
    pub win: u8,
    pub w: u32,
    pub h: u32,
    pub ops: Vec<crate::session::V6Op>,
}
const ENTRY_AUX: &str = "aux.dat";
/// Engine tag (the `EngineSave` engine string) the save was written by.
const ENTRY_ENGINE: &str = "engine.txt";

/// The engine tag stamped into archives written before the tag existed (and the
/// only engine that exists today). Used as the default when an archive has no
/// `engine.txt` entry, so legacy saves restore unchanged.
pub const DEFAULT_ENGINE: &str = "zmachine";

/// The inner save-entry extension for an engine tag, matching the raw
/// interchange extensions: Z-machine Quetzal is `qzl`, Glulx is `glksave`. The
/// archive's `game.<ext>` and per-turn `history/turn-NNNN.<ext>` entries use it.
fn save_ext(engine: &str) -> &'static str {
    if engine == "glulx" { "glksave" } else { "qzl" }
}

/// Whether an archive written by `archive_engine` may be restored while the app
/// is running `current_engine`. The `.lanthorn` archive records the engine that
/// produced the save (see [`ArchiveContents::engine`]); a save from a different
/// engine is refused so a future Glulx save can't be fed to the Z-machine (or
/// vice-versa). Only `"zmachine"` exists today, so this always allows in 3b-i.
pub fn restore_engine_allowed(archive_engine: &str, current_engine: &str) -> Result<(), String> {
    if archive_engine == current_engine {
        Ok(())
    } else {
        Err(format!(
            "this save was written by the \"{archive_engine}\" engine, but lanthorn is running the \"{current_engine}\" engine"
        ))
    }
}
const HISTORY_INDEX: &str = "history/index.json";
/// Prefix for the per-window v6 graphics-canvas PNG blobs (`pictures/win-N.png`).
/// These carry the rasterized `GameSession::pictures_canvas` so a v6 story's
/// frame/graphics windows redraw identically after a host Save State restore
/// (Lane P) — without them a fresh session shows blank graphics windows.
const ENTRY_PICTURES_PREFIX: &str = "pictures/win-";
/// The v6 screen's PAINTED GROUND (SQ-0706): the surface `erase_window` fills and
/// stranded canvases accumulate on, UNDER every window (`GameSession::paint`).
///
/// The last v6 screen layer that a restore did not touch (SQ-0787), which is why a
/// resumed scopa came back showing its main-menu cards beneath the restored game's
/// text: `auto_load` restores after the story has booted and painted its opening
/// screen, and nothing told the ground that screen was gone. Stored as pixels
/// rather than as a recipe on purpose — see `GameSession::paint_ground_png`.
///
/// Not under [`ENTRY_PICTURES_PREFIX`], so the per-window scan never sees it.
const ENTRY_GROUND: &str = "pictures/ground.png";
/// Prefix for the transcript-embedded inline-image PNG blobs
/// (`transcript-img/NNNN.png`, `NNNN` = the filtered transcript-line index). These
/// carry the resolved RGBA pixels of pictures that flow inline with the prose
/// (v6 drop-caps / room icons / content splashes) so a restored transcript shows
/// its art, not just its text (SQ-0518). Sibling metadata is `TranscriptData.images`.
const ENTRY_TRANSCRIPT_IMG_PREFIX: &str = "transcript-img/";

/// Bumped to 8 for SQ-0820: `screen.json` now also carries the other two pixel-run
/// layers of a v6 window — the prose it has STREAMED and the prose a move or resize
/// left RETIRED behind it ([`ZWindowDto::streamed`]/[`ZWindowDto::retired`]). Same
/// break direction as version 7: an older BUILD reading a version-8 archive would
/// drop them and resume fmvpoker with its bet legends missing from the raster, so
/// the version check must reject it (see `load_archive`).
///
/// Version 7 was SQ-0814: `display.json` also carries the v6 screen layers that ride
/// beside the window canvases — the `erase_window` fills and the canvas anchors
/// ([`V6LayersDto`]).
///
/// Version 6 was SQ-0588: a v6 archive carries its display list, and omits the
/// canvas PNG for every window whose replay reproduced the live canvas at save time.
pub const CURRENT_FORMAT_VERSION: u32 = 8;

/// What asked for this archive to be written (SQ-0531). Both triggers produce the
/// SAME `.lanthorn` container — map, transcript, screen, aux and all — so an
/// in-game `@save` is no longer a lesser save. What differs is the *convention* of
/// the `game.<ext>` bytes inside, and it is this field that says which:
///
/// - [`SaveTrigger::Ingame`] — written while the VM is suspended on its own
///   `@save`, so the saved PC sits at the save instruction's branch/store
///   descriptor (Quetzal §5.8). Those bytes are interchange-grade: unzip
///   `game.qzl` and any other interpreter reads it. Restoring one completes the
///   descriptor (`Engine::restore_game_save` / `resume_restore`).
/// - [`SaveTrigger::HostState`] — an emulator-style host snapshot taken *between*
///   turns, so the PC points mid-`read` with no save instruction to store into.
///   Structurally valid Quetzal, but a foreign interpreter would mis-store the
///   restore result, so it is NOT advertised as portable. Restoring one resumes
///   the whole session (`Engine::restore_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveTrigger {
    /// The game's own `@save` (`SAVE` verb) asked for this file.
    Ingame,
    /// The host's Save State (Ctrl+S / `/save-state` / auto-save / exit snapshot).
    #[default]
    HostState,
}

impl SaveTrigger {
    /// Whether the archive's inner game bytes follow the standard save-instruction
    /// PC convention, i.e. can be unzipped and fed to another interpreter.
    pub fn is_portable(self) -> bool {
        matches!(self, SaveTrigger::Ingame)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Meta {
    pub format_version: u32,
    pub ifid: Option<String>,
    /// Human-readable save name, or None for the default (quick-save) slot.
    #[serde(default)]
    pub name: Option<String>,
    /// Turn counter at save time (app-tracked, 0 for saves written before this field existed).
    #[serde(default)]
    pub turns: u32,
    /// RFC3339 timestamp of when this save was written, empty string for legacy saves.
    #[serde(default)]
    pub saved_at: String,
    /// Detected room name at save time, for the picker's save summary (SQ-0411).
    /// None when unknown (legacy saves, or an engine with no location signal).
    #[serde(default)]
    pub location: Option<String>,
    /// Score at save time, from the Z-machine v1–3 automatic status line (SQ-0411).
    /// None for v4+ Z-machine and Glulx, which have no engine-provided score.
    #[serde(default)]
    pub score: Option<i32>,
    /// Which mechanism wrote this archive, and therefore which PC convention the
    /// inner `game.<ext>` bytes follow (SQ-0531). Defaults to
    /// [`SaveTrigger::HostState`] — the only kind that existed before the field.
    #[serde(default)]
    pub trigger: SaveTrigger,
}

/// Transcript payload written to `transcript.json` inside the archive.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TranscriptData {
    lines: Vec<String>,
    kinds: Vec<crate::state::TranscriptKind>,
    /// Per-line Z-machine style runs, parallel to `lines`. Defaults to empty for
    /// archives written before this field existed (back-compatible load).
    #[serde(default)]
    runs: Vec<Vec<crate::state::StyleRun>>,
    /// Per-line Glk paragraph layout format, parallel to `lines` (SQ-0330).
    /// Defaults to empty for archives written before this field existed → the
    /// loader fills left/no-indent defaults.
    #[serde(default)]
    para: Vec<crate::state::ParaFmt>,
    /// Per-line inline-image metadata, parallel to `lines` (SQ-0518). `Some`
    /// marks a line whose logical unit is a transcript-embedded picture (v6
    /// drop-caps / room icons / content splashes); its resolved RGBA pixels live
    /// in a sibling `transcript-img/NNNN.png` blob keyed by this line's index.
    /// Defaults to empty for archives written before this field existed → the
    /// loader restores a transcript with no inline art (acceptable pre-release).
    #[serde(default)]
    images: Vec<Option<InlineImageDto>>,
}

/// serde mirror of the persisted fields of [`crate::inline_image::InlineImage`]
/// (SQ-0518). The `pixels` are NOT stored here — they ride in a separate
/// `transcript-img/NNNN.png` blob (PNG is lossless for RGBA, so a restored inline
/// image reproduces its on-screen pixels exactly, palette state and all).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct InlineImageDto {
    align: crate::inline_image::ImageAlign,
    scaled: Option<(u32, u32)>,
    margin_px: Option<u32>,
}

/// Z-machine screen state written to `screen.json` (zvm has no serde, so we
/// mirror the public fields here). Restored on the host-mediated restore paths
/// (Ctrl+R / auto-load) so a once-split game's upper window shows after restore.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ScreenDto {
    upper_window_rows: u16,
    current_window: u8,
    text_style: u8,
    cursor_row: u16,
    cursor_col: u16,
    buffer_mode: bool,
    show_status_requested: bool,
    cols: u16,
    rows: u16,
    cells: Vec<(char, u8)>, // upper-window grid (ch, style) in row-major order
    /// The full v6 8-window table (geometry, cursors, margins, colours, grids,
    /// pixel-text runs), `Some` only for v6 stories. Serialized so a host Save
    /// State restore reproduces the v6 chrome/status layout exactly (Lane P);
    /// `#[serde(default)]` keeps pre-v6 archives loading as `None`.
    #[serde(default)]
    v6: Option<V6WindowsDto>,
    /// The current logical fg/bg pair (SQ-0551).
    ///
    /// `ScreenState` documents these as transient display state and does not put
    /// them in a Quetzal save — but the PROSE stream tags every run's colour from
    /// them, so a resume that hands them back as `Default` prints the first turn
    /// in the host theme's ink until the game next calls `set_colour`.
    ///
    /// A **v6** story needs no help here: ZMSD §8.3 gives each window its own
    /// pair, the whole window table is archived above, and `restore_screen`
    /// re-derives the current pair from it — so for v6 these fields are written
    /// but ignored on the way back in. Versions 1–5/7/8 have no window table and
    /// nothing else that holds the game's selected colour (this DTO stores the
    /// upper window as char+style only), so for them the pair must travel.
    #[serde(default)]
    current_fg: ZColourDto,
    #[serde(default)]
    current_bg: ZColourDto,
    /// The v6 window the game last asked for INPUT through (SQ-0749). It is an
    /// input to what the screen must show — `BufferWindow::reads_input` derives
    /// straight from it — and Quetzal saves no screen state by design, so this is
    /// ours to carry. Unpersisted, a Save State taken mid-read through a secondary
    /// panel came back with it at 0: the panel's typed-input echo went dark until
    /// the next read re-established it. `#[serde(default)]` keeps pre-SQ-0749
    /// archives loading (as window 0, the pre-existing behaviour).
    #[serde(default)]
    v6_input_window: u8,
}

/// Upper bound on a restored grid's dimensions (SQ-0647). Well past any real
/// terminal (ZMSD §11.1 gives the header only a BYTE each for screen height and
/// width in characters), and low enough that a corrupt `65535 × 65535` cannot ask
/// for a 4-billion-cell allocation on the way in. A restore reconciles the saved
/// screen against the current pane anyway (`reconcile_restored_screen_size`), so
/// clamping here costs a restore nothing it wasn't about to recompute.
const MAX_GRID_COLS: u16 = 1024;
const MAX_GRID_ROWS: u16 = 1024;

/// Build an `UpperWindow` from archived dimensions + cells, enforcing the invariant
/// every consumer assumes: `cells.len() == cols * rows`.
///
/// zvm's grid code (`resize_preserving`, and every `r * cols + c` read after it)
/// indexes straight into `cells`, so a `screen.json` whose vector doesn't match its
/// own dimensions is a panic waiting for the first repaint after the restore — and
/// the archive is a file on the player's disk, not a value we produced this run.
/// Repair rather than reject: the surrounding loader treats a bad `screen.json` as
/// "no saved screen" (the story repaints), but a grid that is merely the wrong
/// length still holds the text that was on screen, so pad or truncate it to fit and
/// keep what's there.
fn grid_from_dto(cols: u16, rows: u16, mut cells: Vec<zvm::screen::Cell>) -> zvm::screen::UpperWindow {
    let cols = cols.min(MAX_GRID_COLS);
    let rows = rows.min(MAX_GRID_ROWS);
    let want = cols as usize * rows as usize;
    if cells.len() != want {
        cells.resize(want, zvm::screen::Cell::default());
    }
    zvm::screen::UpperWindow { cols, rows, cells }
}

impl ScreenDto {
    fn from_screen(s: &zvm::screen::ScreenState) -> Self {
        ScreenDto {
            upper_window_rows: s.upper_window_rows,
            current_window: s.current_window,
            text_style: s.text_style,
            cursor_row: s.cursor_row,
            cursor_col: s.cursor_col,
            buffer_mode: s.buffer_mode,
            show_status_requested: s.show_status_requested,
            cols: s.upper.cols,
            rows: s.upper.rows,
            cells: s.upper.cells.iter().map(|c| (c.ch, c.style)).collect(),
            v6: s.v6.as_ref().map(V6WindowsDto::from_v6),
            // Written ONLY when there is no window table to re-derive from, so
            // each version has exactly one source of truth for its ink and
            // neither mechanism can quietly paper over the other going wrong.
            current_fg: match s.v6 {
                Some(_) => ZColourDto::Default,
                None => ZColourDto::from_z(s.current_fg),
            },
            current_bg: match s.v6 {
                Some(_) => ZColourDto::Default,
                None => ZColourDto::from_z(s.current_bg),
            },
            v6_input_window: s.v6_input_window,
        }
    }

    /// Rebuild the live screen from the DTO, REPAIRING anything the file cannot be
    /// trusted to have got right (SQ-0647). See [`grid_from_dto`]: `screen.json` is a
    /// file on the player's disk, and a truncated or hand-edited one used to hand zvm
    /// a grid whose `cells` didn't match its own `cols × rows`, which
    /// `UpperWindow::resize_preserving` indexes without checking — the first repaint
    /// after the restore panicked the app.
    fn to_screen(&self) -> zvm::screen::ScreenState {
        zvm::screen::ScreenState {
            upper_window_rows: self.upper_window_rows,
            current_window: self.current_window,
            text_style: self.text_style,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            buffer_mode: self.buffer_mode,
            show_status_requested: self.show_status_requested,
            upper: grid_from_dto(
                self.cols,
                self.rows,
                self.cells
                    .iter()
                    .map(|&(ch, style)| zvm::screen::Cell { ch, style, fg: zvm::screen::ZColour::Default, bg: zvm::screen::ZColour::Default })
                    .collect(),
            ),
            v6: self.v6.as_ref().map(V6WindowsDto::to_v6),
            current_fg: self.current_fg.to_z(),
            current_bg: self.current_bg.to_z(),
            v6_input_window: self.v6_input_window,
            ..Default::default()
        }
    }
}

// ── v6 window-table mirror DTOs (zvm has no serde, so we mirror its public
// fields here, matching `ScreenDto`'s style). Restored on the host-mediated
// restore paths so a v6 story's window geometry/status text survive. ────────

/// serde mirror of `zvm::screen::ZColour` (that type is transient display state
/// zvm does not serialize; we persist it for host Save State render fidelity).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
enum ZColourDto {
    #[default]
    Default,
    Standard(u8),
    True(u16),
    True24(u32),
}

impl ZColourDto {
    fn from_z(c: zvm::screen::ZColour) -> Self {
        use zvm::screen::ZColour as Z;
        match c {
            Z::Default => ZColourDto::Default,
            Z::Standard(n) => ZColourDto::Standard(n),
            Z::True(v) => ZColourDto::True(v),
            Z::True24(v) => ZColourDto::True24(v),
        }
    }
    fn to_z(&self) -> zvm::screen::ZColour {
        use zvm::screen::ZColour as Z;
        match self {
            ZColourDto::Default => Z::Default,
            ZColourDto::Standard(n) => Z::Standard(*n),
            ZColourDto::True(v) => Z::True(*v),
            ZColourDto::True24(v) => Z::True24(*v),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GridCellDto { ch: char, style: u8, fg: ZColourDto, bg: ZColourDto }

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct V6TextDto {
    y: u16,
    x: u16,
    text: String,
    style: u8,
    fg: ZColourDto,
    bg: ZColourDto,
    /// The screen character CELL the run's first glyph was written at (SQ-1009).
    ///
    /// Archived rather than re-derived because on a machine that drew
    /// proportionally it CANNOT be re-derived: `(x - 1) / cell.w` is the column
    /// only while the pen advances one declared cell per character, and Arthur's
    /// Amiga press does not. This is the recipe, not the result — a cell backend
    /// places every restored run by it.
    grow: u16,
    gcol: u16,
}

/// serde mirror of one `zvm::screen::ZWindow`. `props` holds the 16 ZMSD window
/// properties (indices 0–15, §8.8.3.2) in field order; the grid, colours and
/// pixel-text runs travel alongside.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ZWindowDto {
    props: [u16; 16],
    cols: u16,
    rows: u16,
    cells: Vec<GridCellDto>,
    fg: ZColourDto,
    bg: ZColourDto,
    texts: Vec<V6TextDto>,
    /// A secondary prose window's live lines (SQ-0585). Persisted for the same
    /// reason as `texts` and the picture canvases: a restore has to reproduce what
    /// was on screen. Measured on advent.z6 — after a restore into its split layout
    /// the game repaints neither window, so an unpersisted panel would come back
    /// blank and stay blank. `#[serde(default)]` keeps older archives loading.
    #[serde(default)]
    prose: Vec<String>,
    /// Where the prose this window has STREAMED to the host transcript is currently
    /// sitting on the screen (SQ-0697/SQ-0729), and…
    #[serde(default)]
    streamed: Vec<V6TextDto>,
    /// …the prose it streamed that a later move or resize FROZE in place (SQ-0697),
    /// at coordinates the window no longer covers.
    ///
    /// Persisted for the same reason as `texts` and `prose` (SQ-0585/SQ-0820): they
    /// are live screen state that only the game repaints, and a host Save State swaps
    /// memory under a game that never learns it happened. Measured on fmvpoker.z6 —
    /// its "Current Bet:"/"10" legends live only here, so an unpersisted `streamed`
    /// brought the game back with them missing from the pixel raster (the cell grid,
    /// which `cells` carries, is why cell mode hid it).
    ///
    /// The RECIPE, not a result: these are the game's own runs in zvm's native pixel
    /// space, exactly as `texts` travels, so the archive stays terminal- and
    /// backend-neutral.
    #[serde(default)]
    retired: Vec<V6TextDto>,
}

/// `Vec<V6Text>` ⇄ `Vec<V6TextDto>`, shared by the three pixel-run layers a v6
/// window carries (`texts`, `streamed`, `retired`).
fn v6_texts_to_dto(runs: &[zvm::screen::V6Text]) -> Vec<V6TextDto> {
    runs.iter()
        .map(|t| V6TextDto {
            y: t.y,
            x: t.x,
            text: t.text.clone(),
            style: t.style,
            fg: ZColourDto::from_z(t.fg),
            bg: ZColourDto::from_z(t.bg),
            grow: t.grow,
            gcol: t.gcol,
        })
        .collect()
}

fn v6_texts_from_dto(runs: &[V6TextDto]) -> Vec<zvm::screen::V6Text> {
    runs.iter()
        .map(|t| zvm::screen::V6Text {
            y: t.y,
            x: t.x,
            text: t.text.clone(),
            style: t.style,
            fg: t.fg.to_z(),
            bg: t.bg.to_z(),
            grow: t.grow,
            gcol: t.gcol,
        })
        .collect()
}

impl ZWindowDto {
    fn from_window(w: &zvm::screen::ZWindow) -> Self {
        let mut props = [0u16; 16];
        for (n, p) in props.iter_mut().enumerate() {
            *p = w.get_prop(n as u16);
        }
        ZWindowDto {
            props,
            cols: w.grid.cols,
            rows: w.grid.rows,
            cells: w.grid.cells.iter().map(|c| GridCellDto {
                ch: c.ch, style: c.style, fg: ZColourDto::from_z(c.fg), bg: ZColourDto::from_z(c.bg),
            }).collect(),
            fg: ZColourDto::from_z(w.fg),
            bg: ZColourDto::from_z(w.bg),
            texts: v6_texts_to_dto(&w.texts),
            prose: w.prose.clone(),
            streamed: v6_texts_to_dto(&w.streamed),
            retired: v6_texts_to_dto(&w.retired),
        }
    }
    fn to_window(&self) -> zvm::screen::ZWindow {
        let mut w = zvm::screen::ZWindow::default();
        for (n, &v) in self.props.iter().enumerate() {
            w.put_prop(n as u16, v);
        }
        // Same repair as the upper window (SQ-0647): a v6 window's grid is indexed by
        // cols/rows too, so the archived cell count has to be made to match.
        w.grid = grid_from_dto(
            self.cols,
            self.rows,
            self.cells.iter().map(|c| zvm::screen::Cell {
                ch: c.ch, style: c.style, fg: c.fg.to_z(), bg: c.bg.to_z(),
            }).collect(),
        );
        w.fg = self.fg.to_z();
        w.bg = self.bg.to_z();
        w.texts = v6_texts_from_dto(&self.texts);
        w.prose = self.prose.clone();
        w.streamed = v6_texts_from_dto(&self.streamed);
        w.retired = v6_texts_from_dto(&self.retired);
        // `stream_origin` is deliberately absent: per-burst state that only lives
        // between a clear and the read that follows it, meaningless across a save.
        w
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct V6WindowsDto { windows: Vec<ZWindowDto>, current: u8 }

impl V6WindowsDto {
    fn from_v6(v: &zvm::screen::V6Windows) -> Self {
        V6WindowsDto { windows: v.windows.iter().map(ZWindowDto::from_window).collect(), current: v.current }
    }
    fn to_v6(&self) -> zvm::screen::V6Windows {
        let mut v = zvm::screen::V6Windows::default();
        for (i, wd) in self.windows.iter().take(8).enumerate() {
            v.windows[i] = wd.to_window();
        }
        // ZMSD §8.4 has exactly eight v6 windows, and `windows[current]` is a fixed
        // array index every `Engine::screen()` call performs — an archived `current`
        // of 9 panicked on the first frame after the restore, not on the load
        // (SQ-0647). Clamp to the last window rather than refusing the archive: the
        // window table it names is still good, and the story selects a window again
        // the moment it draws.
        v.current = self.current.min(7);
        v
    }
}

/// One row of `history/index.json`: per-turn metadata + ordering. The bytes,
/// map JSON, and transcript live in sibling `turn-NNNN.*` entries.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct HistoryIndexEntry {
    turn: u32,
    command: String,
    has_map: bool,
}

#[derive(Debug)]
pub struct ArchiveContents {
    pub mapper: Mapper,
    pub save: Vec<u8>,
    pub meta: Meta,
    /// Console transcript lines (may be empty for archives that pre-date this field).
    pub transcript: Vec<String>,
    /// Parallel kind tag for each transcript entry (same length as `transcript`).
    pub transcript_kinds: Vec<crate::state::TranscriptKind>,
    /// Parallel per-line Z-machine style runs (same length as `transcript`;
    /// empty per line for archives that pre-date this field).
    pub transcript_runs: Vec<Vec<crate::state::StyleRun>>,
    /// Parallel per-line Glk paragraph layout (same length as `transcript`;
    /// left/no-indent default for archives that pre-date this field). (SQ-0330)
    pub transcript_para: Vec<crate::state::ParaFmt>,
    /// Parallel per-line inline image (same length as `transcript`; `None` per
    /// line for archives that pre-date this field). Re-attach to
    /// `AppState::transcript_images` after `reset_transcript_sidecars` at every
    /// restore site so a restored transcript renders its embedded art (SQ-0518).
    pub transcript_images: Vec<Option<crate::inline_image::InlineImage>>,
    /// Per-turn rewind/replay history (empty for archives without `history/`).
    /// `Arc`-wrapped to match `AppState::history` (SQ-1184) — see there.
    pub history: Vec<std::sync::Arc<crate::history::TurnRecord>>,
    /// Saved Z-machine screen state (None for archives without `screen.json`).
    /// Applied on the host-mediated restore paths so the upper window is restored.
    /// For v6 stories this also carries the full 8-window table (`screen.v6`).
    pub screen: Option<zvm::screen::ScreenState>,
    /// The v6 display list + Current Palette (`display.json`), `None` for non-v6
    /// stories and for archives written before SQ-0588. When present it is the
    /// AUTHORITATIVE form of the v6 screen — feed it to
    /// `GameSession::load_display_list`, which rebuilds each window's canvas by
    /// replaying the ops under the restored palette and falls back to `pictures`
    /// for any window the list does not cover.
    pub display: Option<DisplayListDto>,
    /// Per-window v6 graphics-canvas PNG blobs `(window_number, png_bytes)`,
    /// sorted by window number (empty for non-v6 / archives without graphics).
    /// Feed to `GameSession::load_pictures_png` so a restored v6 story redraws
    /// its frame/graphics windows identically (Lane P).
    pub pictures: Vec<(u8, Vec<u8>)>,
    /// The v6 painted ground as a PNG (`pictures/ground.png`, SQ-0787), `None` for
    /// non-v6 stories and for a game that never painted one. Feed to
    /// `GameSession::load_paint_ground`, which RESETS the ground when this is
    /// `None` — a restore must not leave the pre-restore surface standing.
    pub ground: Option<Vec<u8>>,
    /// Auxiliary key/value data from the machine (empty for archives without `aux.dat`).
    pub aux: std::collections::BTreeMap<String, Vec<u8>>,
    /// Shell-style command history (empty for archives without `command_history.json`).
    pub command_history: Vec<String>,
    /// The engine that wrote the save (`engine.txt`), defaulting to
    /// [`DEFAULT_ENGINE`] for archives written before the tag existed. The
    /// restore path refuses a save from a different engine (see
    /// [`restore_engine_allowed`]).
    pub engine: String,
}

/// Everything the SESSION contributes to an archive, as one value.
///
/// **This exists because the alternative lost data in the field.** These seven
/// slices used to be seven positional arguments in the middle of a sixteen-argument
/// writer, five of them `&[…]` of different element types and two of them plain
/// `&[String]`-ish neighbours. `persist_files::save_named` passed `&[], &[]` for
/// the last two under a comment reading "named saves are separate slots; command
/// history is per-game, not per-slot" — which is true of `command_history` and not
/// of `history`. The comment appeared to cover both. So every named Save State
/// wrote an archive with NO rewind/replay history at all: 22 turns played, no
/// `history/` directory in the file, and a restore that came back with nothing to
/// rewind through. Nothing failed; the number was simply absent (SQ-1090).
///
/// The cure the refactoring policy prescribes is a type, and [`of`](Self::of) is
/// the half that matters — a caller that hands over the whole session cannot omit
/// a field of it. A caller that must deviate says so by name, and struct-update
/// syntax keeps the deviation to one line that a reader can see:
///
/// ```ignore
/// SessionRecord { command_history: &[], ..SessionRecord::of(state) }
/// ```
pub struct SessionRecord<'a> {
    pub transcript: &'a [String],
    pub kinds: &'a [crate::state::TranscriptKind],
    pub runs: &'a [Vec<crate::state::StyleRun>],
    pub para: &'a [crate::state::ParaFmt],
    pub images: &'a [Option<crate::inline_image::InlineImage>],
    /// Per-turn rewind/replay records. Empty is a legitimate value — the capture
    /// is opt-in (`record_turn_history`) — which is exactly why an omission here
    /// could not be told apart from a player who had it switched off.
    /// `Arc`-wrapped to match `AppState::history` (SQ-1184) — see there.
    pub history: &'a [std::sync::Arc<crate::history::TurnRecord>],
    pub command_history: &'a [String],
}

impl<'a> SessionRecord<'a> {
    /// The whole session, straight off the live state. The spelling that cannot
    /// forget a field.
    pub fn of(state: &'a crate::state::AppState) -> Self {
        SessionRecord {
            transcript: &state.transcript,
            kinds: &state.transcript_kinds,
            runs: &state.transcript_runs,
            para: &state.transcript_para,
            images: &state.transcript_images,
            history: &state.history,
            command_history: &state.command_history,
        }
    }

    /// A session with nothing in it — for the callers that archive a machine
    /// rather than a play session (tests, and the bare-machine writer).
    pub fn empty() -> Self {
        SessionRecord { transcript: &[], kinds: &[], runs: &[], para: &[], images: &[], history: &[], command_history: &[] }
    }

    /// An owned clone of this session (SQ-1184), for handing to the background
    /// archive worker: it builds and writes after the borrow this was taken
    /// from is gone, so it needs its own copy.
    ///
    /// Defined once, here, alongside the field list itself — the same cure the
    /// module doc for this type prescribes for `SessionRecord` positionally
    /// losing a field (SQ-1090): a future field added to `SessionRecord` is
    /// cloned here too, by construction, rather than by a second hand-maintained
    /// list a caller could omit from.
    pub fn snapshot(&self) -> OwnedSessionRecord {
        OwnedSessionRecord {
            transcript: self.transcript.to_vec(),
            kinds: self.kinds.to_vec(),
            runs: self.runs.to_vec(),
            para: self.para.to_vec(),
            images: self.images.to_vec(),
            history: self.history.to_vec(),
            command_history: self.command_history.to_vec(),
        }
    }
}

/// Owned counterpart to [`SessionRecord`] (SQ-1184): every field cloned out of
/// the borrow so it can cross a thread boundary. `images` and `history` clone
/// cheaply (an `Arc` clone per element) — `transcript`/`kinds`/`runs`/`para` are
/// plain `Vec` clones and stay O(transcript length) on the calling thread; see
/// `archive_worker` for why that is the accepted cost here.
pub struct OwnedSessionRecord {
    pub transcript: Vec<String>,
    pub kinds: Vec<crate::state::TranscriptKind>,
    pub runs: Vec<Vec<crate::state::StyleRun>>,
    pub para: Vec<crate::state::ParaFmt>,
    pub images: Vec<Option<crate::inline_image::InlineImage>>,
    pub history: Vec<std::sync::Arc<crate::history::TurnRecord>>,
    pub command_history: Vec<String>,
}

impl OwnedSessionRecord {
    /// Borrow this back out as a [`SessionRecord`], for feeding to
    /// [`build_archive_bytes`].
    pub fn as_borrowed(&self) -> SessionRecord<'_> {
        SessionRecord {
            transcript: &self.transcript,
            kinds: &self.kinds,
            runs: &self.runs,
            para: &self.para,
            images: &self.images,
            history: &self.history,
            command_history: &self.command_history,
        }
    }
}

/// Write a `.lanthorn` archive containing the current map and VM save.
///
/// `save` is the engine-tagged game state (from `Engine::save_state`); its
/// `bytes` become `game.qzl` (Z-machine) or `game.glksave` (Glulx), and the
/// `engine` tag becomes `engine.txt`. `screen`
/// is the Z-machine `ScreenState` written to `screen.json` — `Some` only for the
/// Z-machine (Glulx keeps its display inside `save.bytes`). `aux` is the engine's
/// auxiliary key/value table.
pub fn save_archive(
    path: &Path,
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    transcript_runs: &[Vec<crate::state::StyleRun>],
    transcript_para: &[crate::state::ParaFmt],
    history: &[std::sync::Arc<crate::history::TurnRecord>],
    command_history: &[String],
) -> io::Result<()> {
    save_archive_meta(path, mapper, save, screen, aux, Meta {
        format_version: CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
        location: None,
        score: None,
        trigger: SaveTrigger::HostState,
    }, transcript, transcript_kinds, transcript_runs, transcript_para, history, command_history)
}

/// Write a `.lanthorn` archive with explicit metadata (name, turns, saved_at).
///
/// Used by `persist_files::save_named` to attach save slot information. See
/// [`save_archive`] for the `save`/`screen`/`aux` parameters. Persists no v6
/// graphics canvases — a thin wrapper over [`save_archive_meta_pics`] for the
/// non-v6 (or graphics-less) callers.
#[allow(clippy::too_many_arguments)]
pub fn save_archive_meta(
    path: &Path,
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    meta: Meta,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    transcript_runs: &[Vec<crate::state::StyleRun>],
    transcript_para: &[crate::state::ParaFmt],
    history: &[std::sync::Arc<crate::history::TurnRecord>],
    command_history: &[String],
) -> io::Result<()> {
    // No pictures, no display list and no painted ground: the non-v6 entry point.
    let session = SessionRecord {
        transcript,
        kinds: transcript_kinds,
        runs: transcript_runs,
        para: transcript_para,
        images: &[],
        history,
        command_history,
    };
    save_archive_meta_pics(path, mapper, save, screen, aux, meta, &session, &[], None, None)
}

/// Write a `.lanthorn` archive including per-window v6 graphics-canvas PNG
/// blobs (`pictures/win-N.png`), the host Save State entry point for v6 stories
/// (Lane P). `pictures` is `(window_number, png_bytes)` — pass
/// `GameSession::pictures_png()`. Non-v6 callers use [`save_archive_meta`],
/// which forwards an empty `pictures`.
///
/// `transcript_images` is the parallel-to-`transcript` inline-image sidecar
/// (`AppState::transcript_images`): the pictures that flow with the prose (v6
/// drop-caps / room icons / content splashes). Each `Some` entry's resolved RGBA
/// is PNG-encoded into a sibling `transcript-img/NNNN.png` blob and its draw
/// metadata into `transcript.json` so a restored transcript renders its embedded
/// art, not just its text (SQ-0518). Pass an empty slice when there is none.
/// As [`save_archive_meta_pics`], plus the v6 DISPLAY LIST and Current Palette
/// (SQ-0588) — what the story *did*, rather than what the screen *looked like*.
///
/// A canvas PNG restores pixels with no draw history, so a restored window cannot
/// be replayed and therefore cannot be recoloured when the game next changes
/// palette (Blorb §11.3); its art keeps the colours it was saved with for the rest
/// of the session. Replaying the ops rebuilds the same screen in a form that still
/// responds to the palette.
///
/// `pictures` stays the per-window FALLBACK, not the default: the caller
/// ([`crate::session::GameSession::display_list`]) replays each window into a
/// scratch canvas at save time and only emits a PNG for the windows whose replay
/// does not reproduce the live canvas. A recording gap then costs one window's
/// worth of stale colours and names itself in a diagnostic, instead of silently
/// restoring a screen we cannot rebuild.
///
/// `ground` is the v6 PAINTED GROUND under all of them
/// ([`crate::session::GameSession::paint_ground_png`], SQ-0787) — pixels, because
/// its inputs are an unbounded stream of `erase_window` fills and there is no
/// bounded recipe to store. `None` when the game has never painted one.
#[allow(clippy::too_many_arguments)]
pub fn save_archive_meta_pics(
    path: &Path,
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    meta: Meta,
    session: &SessionRecord<'_>,
    pictures: &[(u8, Vec<u8>)],
    display: Option<&DisplayListDto>,
    ground: Option<&[u8]>,
) -> io::Result<()> {
    let bytes = build_archive_bytes(mapper, save, screen, aux, &meta, session, pictures, display, ground, None, Some(path), None)?;
    crate::storage::atomic_write(path, &bytes)
}

/// Build the `.lanthorn` archive ZIP in memory, without writing to disk.
///
/// The shared builder behind [`save_archive_meta_pics`] (every synchronous
/// caller, `png_cache: None`) and the background archive worker (SQ-1184,
/// `crate::archive_worker`), which calls this off the main thread with
/// `Some` cache so a stable inline image reuses its prior PNG encode instead
/// of re-compressing every turn — see [`PngBlobCache`].
///
/// `reuse_from`, when given, is the path of the archive this write is about
/// to overwrite (SQ-1202): a retained history turn whose content matches an
/// entry already there is copied straight out of it — compressed bytes, CRC
/// and all — instead of Deflating `r.save`/`map_snapshot`/`transcript` again.
/// See [`raw_copy_turn`] for the identity rule. `history_stats`, when given,
/// receives the raw-copied/encoded counts for the turns this call wrote.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_archive_bytes(
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    meta: &Meta,
    session: &SessionRecord<'_>,
    pictures: &[(u8, Vec<u8>)],
    display: Option<&DisplayListDto>,
    ground: Option<&[u8]>,
    mut png_cache: Option<&mut PngBlobCache>,
    reuse_from: Option<&Path>,
    mut history_stats: Option<&mut HistoryReuseStats>,
) -> io::Result<Vec<u8>> {
    let SessionRecord {
        transcript,
        kinds: transcript_kinds,
        runs: transcript_runs,
        para: transcript_para,
        images: transcript_images,
        history,
        command_history,
    } = *session;
    // Build the whole ZIP in memory, then land it with ONE atomic write
    // (SQ-0644). `File::create(path)` truncated the player's archive before a
    // single byte of the replacement existed — and this is the auto-save path, so
    // it ran every turn, and again on the way out where a 600 ms exit watchdog can
    // kill the process mid-write. Nothing touches the disk until the archive is
    // complete: a crash (or a failed encode) leaves the previous archive readable.
    // Archives are a few MB at most, so holding one in memory costs nothing worth
    // trading a save for.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // map.json — reuse mapper::persist serialization
    let map_json = to_json(mapper);
    zip.start_file(ENTRY_MAP, options)?;
    zip.write_all(map_json.as_bytes())?;

    // game.<ext> — the engine-tagged save bytes (Quetzal): game.qzl for the
    // Z-machine, game.glksave for Glulx, matching the raw interchange extension.
    zip.start_file(format!("game.{}", save_ext(&save.engine)), options)?;
    zip.write_all(&save.bytes)?;

    // engine.txt — the EngineSave tag (which engine produced this save).
    zip.start_file(ENTRY_ENGINE, options)?;
    zip.write_all(save.engine.as_bytes())?;

    // meta.json
    let meta_json =
        serde_json::to_string_pretty(meta).expect("Meta is always serializable");
    zip.start_file(ENTRY_META, options)?;
    zip.write_all(meta_json.as_bytes())?;

    // transcript.json — persist only the story text the player saw (Story + the
    // player's own Input), dropping Meta/Warning lines (slash-command output,
    // diagnostics) that aren't part of the game's narrative.
    use crate::state::TranscriptKind;
    let mut lines: Vec<String> = Vec::new();
    let mut kinds: Vec<TranscriptKind> = Vec::new();
    let mut runs: Vec<Vec<crate::state::StyleRun>> = Vec::new();
    let mut para: Vec<crate::state::ParaFmt> = Vec::new();
    // Per kept-line inline-image metadata (parallel to `lines`); the resolved
    // pixels of each `Some` entry are PNG-encoded into a sibling blob keyed by
    // the filtered line index (SQ-0518).
    let mut images: Vec<Option<InlineImageDto>> = Vec::new();
    let mut image_blobs: Vec<(usize, std::sync::Arc<Vec<u8>>)> = Vec::new();
    for (i, (line, &k)) in transcript.iter().zip(transcript_kinds.iter()).enumerate() {
        if matches!(k, TranscriptKind::Story | TranscriptKind::Input) {
            let fi = lines.len(); // this line's index within the filtered vecs
            lines.push(line.clone());
            kinds.push(k);
            runs.push(transcript_runs.get(i).cloned().unwrap_or_default());
            para.push(transcript_para.get(i).copied().unwrap_or_default());
            match transcript_images.get(i).and_then(|o| o.as_ref()) {
                Some(img) => {
                    // Cached by `Arc::as_ptr` when a cache is passed (the
                    // background worker, SQ-1184) — an image's encoded bytes
                    // never change while its `Arc` lives, so a stable image
                    // reuses its prior encode instead of re-compressing every
                    // turn. Every synchronous caller passes `None` and encodes
                    // fresh, exactly as before.
                    let encoded = match png_cache.as_deref_mut() {
                        Some(cache) => cache.encode(&img.pixels),
                        None => encode_rgba_png(&img.pixels).map(std::sync::Arc::new),
                    };
                    match encoded {
                        Some(png) => {
                            image_blobs.push((fi, png));
                            images.push(Some(InlineImageDto {
                                align: img.align,
                                scaled: img.scaled,
                                margin_px: img.margin_px,
                            }));
                        }
                        None => {
                            // PNG encode failed (never expected for a valid RgbaImage);
                            // drop the picture rather than desync the parallel vecs.
                            images.push(None);
                        }
                    }
                }
                None => images.push(None),
            }
        }
    }
    let td = TranscriptData { lines, kinds, runs, para, images };
    let transcript_json =
        serde_json::to_string_pretty(&td).expect("TranscriptData is always serializable");
    zip.start_file(ENTRY_TRANSCRIPT, options)?;
    zip.write_all(transcript_json.as_bytes())?;

    // transcript-img/NNNN.png — resolved RGBA pixels for each inline transcript
    // image (only when present). PNG is already compressed; store, don't Deflate.
    if !image_blobs.is_empty() {
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (fi, png) in &image_blobs {
            zip.start_file(format!("{ENTRY_TRANSCRIPT_IMG_PREFIX}{fi:04}.png"), stored)?;
            zip.write_all(png.as_slice())?;
        }
    }

    // command_history.json — the player's submitted command lines (JSON array).
    let cmd_history_json = serde_json::to_string_pretty(command_history)
        .expect("Vec<String> is always serializable");
    zip.start_file(ENTRY_COMMAND_HISTORY, options)?;
    zip.write_all(cmd_history_json.as_bytes())?;

    // screen.json — Z-machine screen state (for host-mediated restore redraw).
    // Z-machine-only: Glulx passes `None` (its display lives inside save.bytes).
    if let Some(scr) = screen {
        let screen_json = serde_json::to_string(&ScreenDto::from_screen(scr))
            .expect("ScreenDto is always serializable");
        zip.start_file(ENTRY_SCREEN, options)?;
        zip.write_all(screen_json.as_bytes())?;
    }

    // display.json — the v6 display list + Current Palette (SQ-0588). Absent for
    // non-v6 stories, and for archives written before the list was persisted.
    if let Some(d) = display {
        let display_json =
            serde_json::to_string(d).expect("DisplayListDto is always serializable");
        zip.start_file(ENTRY_DISPLAY, options)?;
        zip.write_all(display_json.as_bytes())?;
    }

    // aux.dat — engine aux data (only when non-empty).
    if !aux.is_empty() {
        zip.start_file(ENTRY_AUX, options)?;
        zip.write_all(&crate::aux_store::encode_aux(aux))?;
    }

    // history/ — per-turn rewind/replay records (only when non-empty).
    if !history.is_empty() {
        let index: Vec<HistoryIndexEntry> = history
            .iter()
            .map(|r| HistoryIndexEntry {
                turn: r.turn,
                command: r.command.clone(),
                has_map: r.map_snapshot.is_some(),
            })
            .collect();
        let index_json =
            serde_json::to_string_pretty(&index).expect("history index serializable");
        zip.start_file(HISTORY_INDEX, options)?;
        zip.write_all(index_json.as_bytes())?;

        let ext = save_ext(&save.engine);

        // Reuse each retained turn's already-Deflated bytes from the archive
        // at `reuse_from` when its content hasn't changed (SQ-1202): a turn's
        // save/map/transcript never change once recorded, so re-compressing
        // one on every write is pure waste once it has been written once.
        // `open_previous_archive` degrades to `None` for anything that isn't
        // a readable zip at that path (absent on the first write, truncated,
        // or simply a different file) — every turn then falls through to the
        // fresh-encode path below, unchanged from before this change.
        let mut prev_zip = reuse_from.and_then(open_previous_archive);

        for r in history {
            let reused = match prev_zip.as_mut() {
                Some(prev) => raw_copy_turn(prev, &mut zip, r, ext)?,
                None => false,
            };
            if reused {
                if let Some(stats) = history_stats.as_deref_mut() {
                    stats.raw_copied += 1;
                }
                continue;
            }
            if let Some(stats) = history_stats.as_deref_mut() {
                stats.encoded += 1;
            }
            zip.start_file(format!("history/turn-{:04}.{}", r.turn, ext), options)?;
            zip.write_all(&r.save)?;
            if let Some(map) = &r.map_snapshot {
                zip.start_file(format!("history/turn-{:04}.map.json", r.turn), options)?;
                zip.write_all(map.as_bytes())?;
            }
            zip.start_file(format!("history/turn-{:04}.txt", r.turn), options)?;
            zip.write_all(r.transcript.as_bytes())?;
        }
    }

    // pictures/win-N.png — v6 per-window graphics canvases (only when present).
    // PNG is already compressed; store without extra Deflate.
    if !pictures.is_empty() {
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (win, png) in pictures {
            zip.start_file(format!("{ENTRY_PICTURES_PREFIX}{win}.png"), stored)?;
            zip.write_all(png)?;
        }
    }

    // pictures/ground.png — the v6 painted ground (SQ-0787), absent when the game
    // has never painted one. Also PNG, so store rather than Deflate.
    if let Some(png) = ground {
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file(ENTRY_GROUND, stored)?;
        zip.write_all(png)?;
    }

    Ok(zip.finish()?.into_inner())
}

/// How many retained history turns one [`build_archive_bytes`] call satisfied
/// by copying compressed bytes out of the previous archive versus
/// re-encoding from scratch (SQ-1202). `Default` is "nothing measured" —
/// every synchronous caller passes `None` for the out-param this fills, so a
/// caller that never asks never pays for tracking it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HistoryReuseStats {
    /// Turns whose `history/turn-NNNN.*` entries were copied verbatim
    /// (compressed bytes, CRC and all) from the previous archive.
    pub raw_copied: usize,
    /// Turns re-encoded from scratch: a fresh write, a turn absent from the
    /// previous archive, or one whose content didn't match it.
    pub encoded: usize,
}

/// Open the archive already at `path` for raw-copy reuse (SQ-1202), or `None`
/// for anything that isn't a readable zip there — absent (the first write to
/// this path), truncated, or simply not a zip. Every history turn then falls
/// through to the ordinary fresh-encode path in [`build_archive_bytes`], so a
/// missing or corrupt previous archive costs nothing beyond the failed open —
/// no error reaches the caller.
fn open_previous_archive(path: &Path) -> Option<zip::ZipArchive<std::fs::File>> {
    let file = std::fs::File::open(path).ok()?;
    zip::ZipArchive::new(file).ok()
}

/// Try to satisfy one retained turn's `history/turn-NNNN.*` entries by
/// copying their compressed bytes straight out of `prev` — via
/// `ZipWriter::raw_copy_file`, which never inflates — instead of re-Deflating
/// `r`'s own bytes (SQ-1202).
///
/// **Identity rule**: reuse is a fact about CONTENT, never about the filename
/// alone. A turn is reused only when EVERY entry it would write this call —
/// `history/turn-NNNN.<ext>` always, `history/turn-NNNN.map.json` exactly
/// when `r.map_snapshot` is `Some`, `history/turn-NNNN.txt` always — is
/// present in `prev` under that SAME name with the SAME uncompressed length
/// AND the SAME CRC-32 ([`entry_matches`]), checked against a CRC-32 this
/// module computes over `r`'s own current bytes without inflating anything
/// from `prev`. `prev` may hold an archive from an unrelated game, an old
/// session, or a different turn count entirely; requiring the save entry
/// (the largest and most specific of the three, a full VM snapshot) to match
/// on top of the turn number already embedded in the name, and the map/
/// transcript siblings to independently match too, is what keeps a
/// coincidentally same-named entry from a different game from ever being
/// reused — the residual risk is exactly a CRC-32 collision on top of an
/// identical length, at identical turn numbers, on every sibling entry at
/// once.
///
/// Checks every sibling's identity BEFORE copying any of them, so a mismatch
/// on the last entry never leaves the first two written into `out` — the
/// caller's fresh-encode fallback always writes a turn's entries from a clean
/// slate, never split between a raw copy of one sibling and a fresh encode of
/// another. Once copying starts, an `Err` (an I/O failure reading `prev` or
/// writing `out`) propagates rather than falling back, matching how every
/// other write in [`build_archive_bytes`] already handles an I/O error —
/// letting the fallback proceed after a partial copy would risk a duplicate
/// entry name in `out`.
fn raw_copy_turn(
    prev: &mut zip::ZipArchive<std::fs::File>,
    out: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
    r: &crate::history::TurnRecord,
    ext: &str,
) -> io::Result<bool> {
    let save_name = format!("history/turn-{:04}.{}", r.turn, ext);
    let map_name = format!("history/turn-{:04}.map.json", r.turn);
    let txt_name = format!("history/turn-{:04}.txt", r.turn);

    if !entry_matches(prev, &save_name, &r.save) {
        return Ok(false);
    }
    if let Some(map) = &r.map_snapshot {
        if !entry_matches(prev, &map_name, map.as_bytes()) {
            return Ok(false);
        }
    }
    if !entry_matches(prev, &txt_name, r.transcript.as_bytes()) {
        return Ok(false);
    }

    // Every applicable sibling matched — safe to copy all of them verbatim.
    let to_io = |e: zip::result::ZipError| io::Error::other(e);
    let f = prev.by_name(&save_name).map_err(to_io)?;
    out.raw_copy_file(f).map_err(to_io)?;
    if r.map_snapshot.is_some() {
        let f = prev.by_name(&map_name).map_err(to_io)?;
        out.raw_copy_file(f).map_err(to_io)?;
    }
    let f = prev.by_name(&txt_name).map_err(to_io)?;
    out.raw_copy_file(f).map_err(to_io)?;
    Ok(true)
}

/// Whether `prev`'s entry `name` has the same uncompressed length and the
/// same CRC-32 as `content` — the per-entry half of [`raw_copy_turn`]'s
/// identity rule. Reads only the zip's stored metadata (from the central
/// directory the read already parsed); never inflates.
fn entry_matches(prev: &mut zip::ZipArchive<std::fs::File>, name: &str, content: &[u8]) -> bool {
    match prev.by_name(name) {
        Ok(f) => f.size() == content.len() as u64 && f.crc32() == crc32(content),
        Err(_) => false,
    }
}

/// CRC-32 (IEEE 802.3 / `zlib`'s, the same polynomial the zip format stores
/// per entry), hand-rolled rather than adding a dependency: `zip` computes
/// its own internally (via `crc32fast`, not part of its public API) but
/// exposes no way to hash arbitrary bytes with it, and this is the only place
/// `app` needs to (SQ-1202) — comparing a turn's CURRENT bytes against a
/// PREVIOUS archive's already-stored checksum, without inflating that entry.
fn crc32(bytes: &[u8]) -> u32 {
    fn table() -> &'static [u32; 256] {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            let mut table = [0u32; 256];
            for (n, entry) in table.iter_mut().enumerate() {
                let mut c = n as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
                *entry = c;
            }
            table
        })
    }
    let table = table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// PNG-encode `pixels`, with no cache — the plain path every synchronous
/// archive caller uses (identical to the pre-SQ-1184 inline behavior). Shared
/// by [`build_archive_bytes`]'s no-cache branch and [`PngBlobCache::encode`]'s
/// miss path, so the two can never disagree on how a blob is produced.
fn encode_rgba_png(pixels: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(pixels.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// PNG-blob cache for inline transcript images, keyed by `Arc::as_ptr` —
/// mirrors `render::inline_image::InlineImageRender`'s cache discipline
/// (SQ-1184): an image's encoded PNG bytes never change while its
/// `Arc<RgbaImage>` lives, so a stable image reuses its prior encode instead
/// of re-compressing every turn. The cached value pins the source `Arc`
/// alongside the bytes, exactly like the render-side cache, so the pointer key
/// can never ABA-collide with a later image that reuses a freed address.
///
/// Lives on the archive worker (`crate::archive_worker`) rather than
/// `AppState`: only the worker thread ever calls [`build_archive_bytes`] with
/// `Some` here, so there is nothing for a synchronous caller to share or
/// invalidate.
#[derive(Default)]
pub struct PngBlobCache {
    cache: std::collections::HashMap<usize, (std::sync::Arc<image::RgbaImage>, std::sync::Arc<Vec<u8>>)>,
}

impl PngBlobCache {
    /// The PNG bytes for `pixels`, encoding and caching on a miss.
    fn encode(&mut self, pixels: &std::sync::Arc<image::RgbaImage>) -> Option<std::sync::Arc<Vec<u8>>> {
        let key = std::sync::Arc::as_ptr(pixels) as usize;
        if let Some((pinned, png)) = self.cache.get(&key) {
            if std::sync::Arc::ptr_eq(pinned, pixels) {
                return Some(std::sync::Arc::clone(png));
            }
        }
        let arc_png = std::sync::Arc::new(encode_rgba_png(pixels)?);
        self.cache.insert(key, (std::sync::Arc::clone(pixels), std::sync::Arc::clone(&arc_png)));
        Some(arc_png)
    }

    /// Evict entries for images no longer live, so a picture that stops
    /// appearing in the transcript doesn't pin its PNG bytes forever. Mirrors
    /// `InlineImageRender::retain_live`.
    pub fn retain_live(&mut self, live: &std::collections::HashSet<usize>) {
        self.cache.retain(|k, _| live.contains(k));
    }
}

/// Read a `.lanthorn` archive.
///
/// Returns `Err` if the file is missing, corrupt, an entry is absent, or
/// `meta.format_version` is greater than `CURRENT_FORMAT_VERSION`. The caller
/// restores the VM save via `machine.restore_file(&contents.save)`.
pub fn load_archive(path: &Path) -> io::Result<ArchiveContents> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // meta.json — check version first
    let meta: Meta = {
        let mut entry = zip.by_name(ENTRY_META).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_META}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_META}: {e}")))?
    };

    if meta.format_version > CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected <= {}",
                meta.format_version, CURRENT_FORMAT_VERSION
            ),
        ));
    }

    // map.json
    let mapper = {
        let mut entry = zip.by_name(ENTRY_MAP).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_MAP}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        from_json(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_MAP}: {e}")))?
    };

    // engine.txt — which engine produced this save; drives the save entry name
    // (game.qzl / game.glksave). Absent → default engine.
    let engine = match zip.by_name(ENTRY_ENGINE) {
        Ok(mut entry) => {
            let mut buf = String::new();
            let _ = entry.read_to_string(&mut buf);
            let t = buf.trim();
            if t.is_empty() { DEFAULT_ENGINE.to_string() } else { t.to_string() }
        }
        Err(_) => DEFAULT_ENGINE.to_string(),
    };

    // game.<ext> — the engine-tagged save bytes.
    let save = {
        let name = format!("game.{}", save_ext(&engine));
        let mut entry = zip.by_name(&name).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}: {e}"))
        })?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };

    // transcript.json — optional; older archives omit it, default to empty vecs.
    // `image_dtos` is the per-line inline-image metadata (SQ-0518); the pixels are
    // read from the sibling `transcript-img/NNNN.png` blobs below.
    let (transcript, transcript_kinds, transcript_runs, transcript_para, image_dtos) = match zip.by_name(ENTRY_TRANSCRIPT) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            match serde_json::from_str::<TranscriptData>(&buf) {
                Ok(td) => {
                    // Keep runs/para length-synced with lines: archives that
                    // pre-date these fields (or with a mismatched length) get one
                    // empty run vec / default ParaFmt per line.
                    let runs = if td.runs.len() == td.lines.len() {
                        td.runs
                    } else {
                        vec![Vec::new(); td.lines.len()]
                    };
                    let para = if td.para.len() == td.lines.len() {
                        td.para
                    } else {
                        vec![crate::state::ParaFmt::default(); td.lines.len()]
                    };
                    let images = if td.images.len() == td.lines.len() {
                        td.images
                    } else {
                        (0..td.lines.len()).map(|_| None).collect()
                    };
                    (td.lines, td.kinds, runs, para, images)
                }
                Err(_) => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            }
        }
        Err(_) => (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };

    // transcript-img/NNNN.png — resolved pixels for the inline transcript images
    // (absent for text-only transcripts). Materialize `transcript_images` parallel
    // to `transcript`: a line's picture needs both a `Some` DTO and a decodable
    // blob at its index (SQ-0518). Collect the blob bytes first (releasing each
    // borrow), then rebuild alongside the DTO metadata.
    let mut img_png: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    {
        let names: Vec<(usize, String)> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
            .filter_map(|name| {
                let n = name.strip_prefix(ENTRY_TRANSCRIPT_IMG_PREFIX)?.strip_suffix(".png")?;
                n.parse::<usize>().ok().map(|idx| (idx, name))
            })
            .collect();
        for (idx, name) in names {
            if let Ok(mut e) = zip.by_name(&name) {
                let mut buf = Vec::new();
                if e.read_to_end(&mut buf).is_ok() {
                    img_png.insert(idx, buf);
                }
            }
        }
    }
    let transcript_images: Vec<Option<crate::inline_image::InlineImage>> = image_dtos
        .iter()
        .enumerate()
        .map(|(i, dto)| {
            let dto = dto.as_ref()?;
            let png = img_png.get(&i)?;
            let rgba = image::load_from_memory(png).ok()?.to_rgba8();
            Some(crate::inline_image::InlineImage {
                pixels: std::sync::Arc::new(rgba),
                align: dto.align,
                scaled: dto.scaled,
                margin_px: dto.margin_px,
            })
        })
        .collect();

    // command_history.json — optional; older archives omit it → empty vec.
    let command_history: Vec<String> = match zip.by_name(ENTRY_COMMAND_HISTORY) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            serde_json::from_str(&buf).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };

    // history/ — optional; absent in archives that pre-date this feature.
    // Read the index first (releasing its borrow before the per-turn reads).
    let history_index: Option<Vec<HistoryIndexEntry>> = match zip.by_name(HISTORY_INDEX) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            Some(serde_json::from_str(&buf).unwrap_or_default())
        }
        Err(_) => None,
    };
    let history: Vec<std::sync::Arc<crate::history::TurnRecord>> = match history_index {
        Some(index) => {
            let mut out = Vec::with_capacity(index.len());
            for e in index {
                let save = {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.{}", e.turn, save_ext(&engine))) {
                        let _ = z.read_to_end(&mut b);
                    }
                    b
                };
                let map_snapshot = if e.has_map {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.map.json", e.turn)) {
                        let _ = z.read_to_end(&mut b);
                    }
                    String::from_utf8(b).ok()
                } else {
                    None
                };
                let transcript = {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.txt", e.turn)) {
                        let _ = z.read_to_end(&mut b);
                    }
                    String::from_utf8(b).unwrap_or_default()
                };
                out.push(std::sync::Arc::new(crate::history::TurnRecord {
                    turn: e.turn,
                    command: e.command,
                    save,
                    map_snapshot,
                    transcript,
                }));
            }
            out
        }
        None => Vec::new(),
    };

    // screen.json — saved Z-machine screen state (absent in pre-screen archives).
    let screen = {
        let mut b = Vec::new();
        if let Ok(mut z) = zip.by_name(ENTRY_SCREEN) {
            if z.read_to_end(&mut b).is_ok() {
                serde_json::from_slice::<ScreenDto>(&b).ok().map(|d| d.to_screen())
            } else {
                None
            }
        } else {
            None
        }
    };

    // aux.dat — optional; absent in pre-aux archives → empty map.
    let aux = match zip.by_name(ENTRY_AUX) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            let _ = entry.read_to_end(&mut buf);
            crate::aux_store::decode_aux(&buf)
        }
        Err(_) => std::collections::BTreeMap::new(),
    };

    // display.json — the v6 display list + Current Palette (SQ-0588). Absent for
    // non-v6 stories and for archives written before it existed; those restore
    // from `pictures` below, exactly as they always did.
    let display: Option<DisplayListDto> = match zip.by_name(ENTRY_DISPLAY) {
        Ok(mut entry) => {
            let mut s = String::new();
            entry.read_to_string(&mut s).ok().and_then(|_| serde_json::from_str(&s).ok())
        }
        Err(_) => None,
    };

    // pictures/win-N.png — v6 per-window graphics canvases (absent for non-v6).
    // Collect the matching names first (releases each borrow before the reads).
    // ZIP central-directory (by_index) order == write order, which is the paint
    // order `pictures_png` emits (ascending z_seq) — preserved here, NOT sorted
    // by window, so `load_pictures_png` can reproduce the relative z-order.
    let picture_names: Vec<(u8, String)> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
        .filter_map(|name| {
            let n = name.strip_prefix(ENTRY_PICTURES_PREFIX)?.strip_suffix(".png")?;
            n.parse::<u8>().ok().map(|win| (win, name))
        })
        .collect();
    let mut pictures: Vec<(u8, Vec<u8>)> = Vec::with_capacity(picture_names.len());
    for (win, name) in picture_names {
        if let Ok(mut e) = zip.by_name(&name) {
            let mut buf = Vec::new();
            if e.read_to_end(&mut buf).is_ok() {
                pictures.push((win, buf));
            }
        }
    }

    // pictures/ground.png — the v6 painted ground (SQ-0787). Absent for non-v6
    // stories, for games that never paint one, and for archives written before it
    // was carried; all three restore to an EMPTY ground rather than to whatever
    // the pre-restore screen happened to be holding.
    let ground: Option<Vec<u8>> = zip.by_name(ENTRY_GROUND).ok().and_then(|mut e| {
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).ok().map(|_| buf)
    });

    Ok(ArchiveContents { mapper, save, meta, transcript, transcript_kinds, transcript_runs, transcript_para, transcript_images, history, screen, aux, command_history, engine, display, pictures, ground })
}

/// Read ONLY the `meta.json` entry from a save archive — avoids `load_archive`
/// unzipping the map, save image, transcript, history, screen, and aux just to
/// show a save summary. Applies the same `format_version` rejection as
/// `load_archive`, so a future-format archive is reported as an error (and thus
/// skipped by `list_saves`) exactly as today.
pub fn read_archive_meta(path: &Path) -> io::Result<Meta> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let meta: Meta = {
        let mut entry = zip.by_name(ENTRY_META).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_META}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_META}: {e}"))
        })?
    };
    if meta.format_version > CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected <= {}",
                meta.format_version, CURRENT_FORMAT_VERSION
            ),
        ));
    }
    Ok(meta)
}

impl ArchiveContents {
    /// The persisted game state as an engine-tagged [`EngineSave`], rebuilt from
    /// the archive's `game.<ext>` bytes + `engine.txt` tag (defaulting to
    /// [`DEFAULT_ENGINE`] for legacy archives). The save-format version is not
    /// stored in the archive — the engine ignores it on restore — so a
    /// placeholder is used. Feed this to `Engine::restore_state`, which refuses a
    /// foreign-engine save via [`restore_engine_allowed`]'s equivalent guard.
    pub fn engine_save(&self) -> EngineSave {
        EngineSave::new(self.engine.clone(), 1, self.save.clone())
    }
}

/// Read raw Quetzal bytes from a save file for an in-game RESTORE.
///
/// If `path` is a `.lanthorn` ZIP archive, returns its `game.qzl`/`game.glksave`
/// entry;
/// otherwise returns the file's raw bytes (a plain `.qzl` Quetzal save).
pub fn read_quetzal_from_file(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        for name in ["game.qzl", "game.glksave"] {
            if let Ok(mut entry) = zip.by_name(name) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                return Ok(buf);
            }
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    fn temp_archive_path(tag: &str) -> std::path::PathBuf {
        crate::scratch_dir(&format!("archive-test-{tag}")).join("save.lanthorn")
    }

    fn small_mapper() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m
    }

    /// The Z-machine `EngineSave` for `machine` (Quetzal bytes, `"zmachine"` tag).
    fn zvm_es(machine: &zvm::cpu::exec::Machine) -> EngineSave {
        EngineSave::new(DEFAULT_ENGINE, 1, machine.save_quetzal())
    }

    /// Test helper: write an archive from a Z-machine `machine` (the old call
    /// shape), building the `EngineSave` + screen + aux the production fn now
    /// takes separately.
    #[allow(clippy::too_many_arguments)]
    fn save_archive_m(
        path: &Path,
        mapper: &Mapper,
        machine: &zvm::cpu::exec::Machine,
        transcript: &[String],
        kinds: &[crate::state::TranscriptKind],
        runs: &[Vec<crate::state::StyleRun>],
        history: &[std::sync::Arc<crate::history::TurnRecord>],
        cmds: &[String],
    ) -> io::Result<()> {
        save_archive(path, mapper, &zvm_es(machine), Some(&machine.screen), &machine.aux_data,
            transcript, kinds, runs, &[], history, cmds)
    }

    #[test]
    fn read_quetzal_extracts_game_sav_from_lanthorn() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let machine = dummy_machine();
        let expected = machine.save_quetzal();

        let path = temp_archive_path("qzl-from-lanthorn");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save_archive");
        let got = read_quetzal_from_file(&path).expect("read_quetzal_from_file");
        let _ = std::fs::remove_file(&path);

        assert_eq!(got, expected, "game.qzl bytes extracted from the .lanthorn");
    }

    #[test]
    fn read_quetzal_returns_raw_bytes_for_plain_qzl() {
        // A non-zip file (a plain .qzl) returns its raw bytes unchanged.
        let path = temp_archive_path("plain-qzl");
        std::fs::write(&path, b"FORM\x00\x00fake-quetzal").unwrap();
        let got = read_quetzal_from_file(&path).expect("read raw");
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, b"FORM\x00\x00fake-quetzal");
    }

    #[test]
    fn restore_engine_allowed_refuses_foreign_tag() {
        // The running engine restoring its own save is allowed.
        assert!(restore_engine_allowed(DEFAULT_ENGINE, DEFAULT_ENGINE).is_ok());
        // A save written by a different (faked) engine is refused.
        let err = restore_engine_allowed("glulx", DEFAULT_ENGINE)
            .expect_err("a foreign-engine save must be refused");
        assert!(err.contains("glulx") && err.contains(DEFAULT_ENGINE), "message names both engines: {err}");
    }

    #[test]
    fn archive_records_and_loads_engine_tag() {
        let machine = dummy_machine();
        let path = temp_archive_path("engine-tag");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE, "archive records the zmachine engine tag");
        assert!(restore_engine_allowed(&ac.engine, DEFAULT_ENGINE).is_ok());
    }

    fn dummy_machine() -> zvm::cpu::exec::Machine {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let story = std::fs::read(&fixture).expect("czech.z5 fixture for archive tests");
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut m = zvm::cpu::exec::Machine::new(mem);
        m.init_caps();
        m
    }

    #[test]
    fn inline_transcript_images_round_trip_through_archive() {
        use crate::inline_image::{ImageAlign, InlineImage};
        use crate::state::TranscriptKind;

        // Two Story lines; the SECOND carries an inline image (an empty-string
        // placeholder line, as `push_transcript_image` makes it). A distinctive
        // gradient so a lossy round-trip would be caught.
        let mut pixels = image::RgbaImage::new(6, 4);
        for (x, y, p) in pixels.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 40) as u8, (y * 60) as u8, 7, 255]);
        }
        let img = InlineImage {
            pixels: std::sync::Arc::new(pixels),
            align: ImageAlign::MarginLeft,
            scaled: Some((12, 8)),
            margin_px: Some(40),
        };

        let transcript = vec!["West of House".to_string(), String::new()];
        let kinds = vec![TranscriptKind::Story, TranscriptKind::Story];
        let images = vec![None, Some(img.clone())];

        let path = temp_archive_path("inline-img-rt");
        let machine = dummy_machine();
        save_archive_meta_pics(
            &path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen), &machine.aux_data,
            Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState },
            &SessionRecord { transcript: &transcript, kinds: &kinds, images: &images, ..SessionRecord::empty() },
            &[],
            None,
            None,
        )
        .expect("save with inline image");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(ac.transcript_images.len(), ac.transcript.len(), "images parallel to transcript");
        assert!(ac.transcript_images[0].is_none(), "text line carries no image");
        let got = ac.transcript_images[1].as_ref().expect("inline image restored on line 1");
        assert_eq!(got.align, ImageAlign::MarginLeft, "align round-trips");
        assert_eq!(got.scaled, Some((12, 8)), "scaled round-trips");
        assert_eq!(got.margin_px, Some(40), "margin_px round-trips");
        assert_eq!(got.pixels.dimensions(), (6, 4), "pixel dims round-trip");
        assert_eq!(
            got.pixels.as_raw(), img.pixels.as_raw(),
            "PNG is lossless — restored pixels are byte-identical to the saved ones"
        );
    }

    #[test]
    fn archive_without_inline_images_loads_all_none() {
        // A transcript with no inline images restores a per-line all-`None` sidecar
        // (and no `transcript-img/` blobs are written).
        let transcript = vec!["West of House".to_string(), "> look".to_string()];
        let kinds = vec![crate::state::TranscriptKind::Story, crate::state::TranscriptKind::Input];
        let path = temp_archive_path("no-inline-img");
        let machine = dummy_machine();
        save_archive_meta_pics(
            &path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen), &machine.aux_data,
            Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState },
            &SessionRecord { transcript: &transcript, kinds: &kinds, ..SessionRecord::empty() },
            &[],
            None,
            None,
        )
        .expect("save without inline images");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.transcript_images.len(), ac.transcript.len());
        assert!(ac.transcript_images.iter().all(Option::is_none), "no images → all None");
    }

    #[test]
    fn screen_state_round_trips_through_archive() {
        let mut machine = dummy_machine();
        machine.screen.upper_window_rows = 1;
        machine.screen.upper.resize(1, 6);
        machine.screen.upper.put(1, 2, 'Z', 2, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        machine.screen.current_window = 1;
        machine.screen.cursor_col = 3;

        let path = temp_archive_path("screen-roundtrip");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        let scr = ac.screen.expect("screen.json present and restored");
        assert_eq!(scr.upper_window_rows, 1, "split height round-trips");
        assert_eq!(scr.current_window, 1, "current window round-trips");
        assert_eq!(scr.cursor_col, 3, "cursor round-trips");
        assert_eq!(scr.upper.cell(1, 2).ch, 'Z', "grid glyph round-trips");
        assert_eq!(scr.upper.cell(1, 2).style, 2, "grid style round-trips");
    }

    #[test]
    fn save_drops_meta_and_warning_transcript_lines() {
        use crate::state::TranscriptKind;

        let transcript = vec![
            "West of House".to_string(),
            "> open mailbox".to_string(),
            "/help".to_string(),
            "save failed".to_string(),
            "You open the mailbox.".to_string(),
        ];
        let kinds = vec![
            TranscriptKind::Story,
            TranscriptKind::Input,
            TranscriptKind::Meta,
            TranscriptKind::Warning,
            TranscriptKind::Story,
        ];

        let path = temp_archive_path("transcript-filter");
        let machine = dummy_machine();
        save_archive_meta(
            &path,
            &small_mapper(),
            &zvm_es(&machine),
            Some(&machine.screen),
            &machine.aux_data,
            Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState },
            &transcript,
            &kinds,
            &[],
            &[],
            &[],
            &[],
        )
        .expect("save_archive_meta");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        // Only Story + Input survive; Meta/Warning are dropped.
        assert_eq!(
            ac.transcript,
            vec!["West of House", "> open mailbox", "You open the mailbox."]
        );
        assert_eq!(
            ac.transcript_kinds,
            vec![TranscriptKind::Story, TranscriptKind::Input, TranscriptKind::Story]
        );
    }

    #[test]
    fn history_round_trips_in_archive() {
        use crate::history::TurnRecord;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        let mapper = small_mapper();
        let map_json = mapper::persist::to_json(&mapper);
        let history = vec![
            std::sync::Arc::new(TurnRecord { turn: 1, command: "look".into(), save: vec![1, 2, 3],
                map_snapshot: Some(map_json.clone()), transcript: "West of House".into() }),
            std::sync::Arc::new(TurnRecord { turn: 2, command: "wait".into(), save: vec![4, 5, 6, 7],
                map_snapshot: None, transcript: "Time passes.".into() }),
        ];

        let path = temp_archive_path("history-rt");
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &history, &[])
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        assert_eq!(ac.history.len(), 2);
        assert_eq!(ac.history[0].turn, 1);
        assert_eq!(ac.history[0].command, "look");
        assert_eq!(ac.history[0].save, vec![1, 2, 3], "save bytes byte-identical");
        assert_eq!(ac.history[0].map_snapshot.as_deref(), Some(map_json.as_str()));
        assert_eq!(ac.history[0].transcript, "West of House");
        assert_eq!(ac.history[1].save, vec![4, 5, 6, 7]);
        assert!(ac.history[1].map_snapshot.is_none(), "no-change turn has no map");
        assert_eq!(ac.history[1].transcript, "Time passes.");
    }

    #[test]
    fn v1_archive_loads_with_empty_history() {
        // An archive with no history/ entries (e.g. written before this feature)
        // loads with an empty history and unchanged behavior.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let mapper = small_mapper();
        let path = temp_archive_path("history-v1");
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &[], &[])
            .expect("save_archive without history");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        assert!(ac.history.is_empty(), "archive without history/ → empty history");
    }

    #[test]
    fn command_history_round_trips_in_archive() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let cmds = vec!["look".to_string(), "open mailbox".to_string(), "/help".to_string()];
        let path = temp_archive_path("cmd-history-rt");
        save_archive_m(&path, &small_mapper(), &dummy_machine(), &[], &[], &[], &[], &cmds)
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.command_history, cmds);
    }

    #[test]
    fn archive_without_command_history_loads_empty() {
        // Simulate a pre-feature archive by removing the command_history.json entry.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let path = temp_archive_path("cmd-history-missing");
        save_archive_m(&path, &small_mapper(), &dummy_machine(), &[], &[], &[], &[],
            &["north".to_string()])
            .expect("save_archive");

        // Rewrite the archive dropping the command_history.json entry.
        let stripped = temp_archive_path("cmd-history-stripped");
        {
            let src = std::fs::File::open(&path).unwrap();
            let mut zin = zip::ZipArchive::new(src).unwrap();
            let out = std::fs::File::create(&stripped).unwrap();
            let mut zout = zip::ZipWriter::new(out);
            for i in 0..zin.len() {
                let mut e = zin.by_index(i).unwrap();
                if e.name() == ENTRY_COMMAND_HISTORY {
                    continue;
                }
                let name = e.name().to_string();
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                zout.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                zout.write_all(&buf).unwrap();
            }
            zout.finish().unwrap();
        }
        let ac = load_archive(&stripped).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&stripped);
        assert!(ac.command_history.is_empty(), "missing entry → empty command history");
    }

    // -------------------------------------------------------------------------
    // history turn reuse across archive writes (SQ-1202)
    // -------------------------------------------------------------------------

    fn empty_meta() -> Meta {
        Meta {
            format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0,
            saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState,
        }
    }

    /// A second write of the SAME unchanged turns raw-copies every one of
    /// them, and only a genuinely new turn is encoded fresh. Falsifies
    /// before the fix: without `reuse_from`/`raw_copy_turn`, every turn is
    /// re-Deflated every write and `raw_copied` never leaves 0.
    #[test]
    fn history_turns_are_raw_copied_on_a_second_write() {
        use crate::history::TurnRecord;

        let mapper = small_mapper();
        let machine = dummy_machine();
        let es = zvm_es(&machine);
        let meta = empty_meta();
        let history1: Vec<std::sync::Arc<TurnRecord>> = vec![
            std::sync::Arc::new(TurnRecord {
                turn: 1, command: "look".into(), save: vec![1, 2, 3],
                map_snapshot: None, transcript: "West of House".into(),
            }),
            std::sync::Arc::new(TurnRecord {
                turn: 2, command: "wait".into(), save: vec![4, 5, 6, 7],
                map_snapshot: None, transcript: "Time passes.".into(),
            }),
        ];
        let path = temp_archive_path("reuse-basic");

        // First write: nothing at `path` yet, so nothing to reuse from.
        let session1 = SessionRecord { history: &history1, ..SessionRecord::empty() };
        let mut stats1 = HistoryReuseStats::default();
        let bytes1 = build_archive_bytes(
            &mapper, &es, Some(&machine.screen), &machine.aux_data, &meta, &session1,
            &[], None, None, None, Some(&path), Some(&mut stats1),
        ).expect("first build");
        crate::storage::atomic_write(&path, &bytes1).expect("first write");
        assert_eq!(
            stats1, HistoryReuseStats { raw_copied: 0, encoded: 2 },
            "nothing to reuse on the very first write"
        );

        // Second write: the same two turns, unchanged, plus one genuinely new turn.
        let history2: Vec<std::sync::Arc<TurnRecord>> = vec![
            history1[0].clone(),
            history1[1].clone(),
            std::sync::Arc::new(TurnRecord {
                turn: 3, command: "north".into(), save: vec![8, 9],
                map_snapshot: None, transcript: "Forest".into(),
            }),
        ];
        let session2 = SessionRecord { history: &history2, ..SessionRecord::empty() };
        let mut stats2 = HistoryReuseStats::default();
        let bytes2 = build_archive_bytes(
            &mapper, &es, Some(&machine.screen), &machine.aux_data, &meta, &session2,
            &[], None, None, None, Some(&path), Some(&mut stats2),
        ).expect("second build");
        assert_eq!(
            stats2, HistoryReuseStats { raw_copied: 2, encoded: 1 },
            "the two unchanged turns are raw-copied; only turn 3 is encoded"
        );
        crate::storage::atomic_write(&path, &bytes2).expect("second write");

        // The archive built via reuse loads with the SAME content a from-scratch
        // encode would produce — the raw copy changed HOW the bytes got there,
        // not WHAT they say.
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.history.len(), 3);
        assert_eq!(ac.history[0].save, vec![1, 2, 3]);
        assert_eq!(ac.history[0].transcript, "West of House");
        assert_eq!(ac.history[1].save, vec![4, 5, 6, 7]);
        assert_eq!(ac.history[1].transcript, "Time passes.");
        assert_eq!(ac.history[2].save, vec![8, 9], "the freshly-encoded turn also round-trips");
        assert_eq!(ac.history[2].transcript, "Forest");

        // And it matches a from-scratch encode of the SAME session bit-for-bit
        // in content: build again with no `reuse_from` and compare inflated
        // history entries.
        let mut fresh_stats = HistoryReuseStats::default();
        let fresh_bytes = build_archive_bytes(
            &mapper, &es, Some(&machine.screen), &machine.aux_data, &meta, &session2,
            &[], None, None, None, None, Some(&mut fresh_stats),
        ).expect("fresh build");
        assert_eq!(fresh_stats, HistoryReuseStats { raw_copied: 0, encoded: 3 });
        let fresh_path = temp_archive_path("reuse-basic-fresh");
        crate::storage::atomic_write(&fresh_path, &fresh_bytes).expect("write fresh");
        let fresh_ac = load_archive(&fresh_path).expect("load fresh");
        let _ = std::fs::remove_file(&fresh_path);
        for i in 0..3 {
            assert_eq!(ac.history[i].save, fresh_ac.history[i].save, "turn {i} save bytes match a from-scratch encode");
            assert_eq!(ac.history[i].transcript, fresh_ac.history[i].transcript, "turn {i} transcript matches a from-scratch encode");
            assert_eq!(ac.history[i].command, fresh_ac.history[i].command, "turn {i} command matches a from-scratch encode");
        }
    }

    /// A previous archive that is absent, corrupt, or belongs to an unrelated
    /// session/game degrades to a full re-encode with no error — the identity
    /// rule (turn name + uncompressed length + CRC-32, all three sibling
    /// entries) refuses a same-named entry whose content doesn't match.
    #[test]
    fn history_reuse_degrades_gracefully_when_previous_archive_is_unusable() {
        use crate::history::TurnRecord;

        let mapper = small_mapper();
        let machine = dummy_machine();
        let es = zvm_es(&machine);
        let meta = empty_meta();
        let history: Vec<std::sync::Arc<TurnRecord>> = vec![std::sync::Arc::new(TurnRecord {
            turn: 1, command: "look".into(), save: vec![9, 9, 9],
            map_snapshot: None, transcript: "A room.".into(),
        })];
        let session = SessionRecord { history: &history, ..SessionRecord::empty() };

        // Case 1: absent — nothing was ever written to this path.
        let absent_path = temp_archive_path("reuse-absent");
        let mut stats_absent = HistoryReuseStats::default();
        let bytes_absent = build_archive_bytes(
            &mapper, &es, None, &machine.aux_data, &meta, &session,
            &[], None, None, None, Some(&absent_path), Some(&mut stats_absent),
        ).expect("build with an absent previous archive must not error");
        assert_eq!(stats_absent, HistoryReuseStats { raw_copied: 0, encoded: 1 });

        // Case 2: truncated / not a zip at all.
        let truncated_path = temp_archive_path("reuse-truncated");
        std::fs::create_dir_all(truncated_path.parent().unwrap()).unwrap();
        std::fs::write(&truncated_path, b"not a zip file").unwrap();
        let mut stats_truncated = HistoryReuseStats::default();
        let bytes_truncated = build_archive_bytes(
            &mapper, &es, None, &machine.aux_data, &meta, &session,
            &[], None, None, None, Some(&truncated_path), Some(&mut stats_truncated),
        ).expect("build with a corrupt previous archive must not error");
        assert_eq!(stats_truncated, HistoryReuseStats { raw_copied: 0, encoded: 1 });

        // Case 3: a readable archive at the same path, same turn number, but
        // DIFFERENT content — a different game/session, not a stale copy of
        // this one. Must never be mistaken for a match.
        let other_path = temp_archive_path("reuse-different-session");
        let other_history: Vec<std::sync::Arc<TurnRecord>> = vec![std::sync::Arc::new(TurnRecord {
            turn: 1, command: "xyzzy".into(), save: vec![1, 1, 1, 1, 1],
            map_snapshot: None, transcript: "Somewhere else entirely, a long way from here.".into(),
        })];
        let other_session = SessionRecord { history: &other_history, ..SessionRecord::empty() };
        let other_bytes = build_archive_bytes(
            &mapper, &es, None, &machine.aux_data, &meta, &other_session,
            &[], None, None, None, None, None,
        ).expect("build unrelated archive");
        crate::storage::atomic_write(&other_path, &other_bytes).expect("write unrelated archive");
        let mut stats_other = HistoryReuseStats::default();
        let bytes_other = build_archive_bytes(
            &mapper, &es, None, &machine.aux_data, &meta, &session,
            &[], None, None, None, Some(&other_path), Some(&mut stats_other),
        ).expect("build against an unrelated archive at the same path must not error");
        assert_eq!(
            stats_other, HistoryReuseStats { raw_copied: 0, encoded: 1 },
            "same turn number, different content — never reused"
        );

        // All three fall back to a correct output: writing and loading each
        // reproduces this session's own turn 1, not the unrelated one.
        for (path, bytes) in [
            (&absent_path, &bytes_absent),
            (&truncated_path, &bytes_truncated),
            (&other_path, &bytes_other),
        ] {
            crate::storage::atomic_write(path, bytes).expect("write");
            let ac = load_archive(path).expect("load");
            assert_eq!(ac.history.len(), 1);
            assert_eq!(ac.history[0].save, vec![9, 9, 9], "this session's own turn 1, not the unrelated one");
            assert_eq!(ac.history[0].transcript, "A room.");
            let _ = std::fs::remove_file(path);
        }
    }

    /// A restore off a RAW-COPIED archive must actually work, not merely
    /// load: rewind to the first of two raw-copied turns, then replay forward
    /// to the second, restoring each turn's Quetzal snapshot into a real
    /// `Machine` (CLAUDE.md: "restore tests must perturb before asserting" —
    /// a raw copy that quietly corrupted CRC or byte range wouldn't show up
    /// in `ac.history[i].save == expected` alone if a bad copy happened to
    /// come back the same length, but WOULD show up as a Quetzal restore
    /// failure here).
    #[test]
    fn raw_copied_archive_round_trips_through_rewind_and_replay() {
        use crate::history::TurnRecord;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let story = std::fs::read(&fixture).expect("czech.z5 fixture");

        let mut m1 = dummy_machine();
        for _ in 0..3 { let _ = m1.step(); }
        let save1 = m1.save_quetzal();

        let mut m2 = dummy_machine();
        for _ in 0..7 { let _ = m2.step(); }
        let save2 = m2.save_quetzal();

        let mapper = small_mapper();
        let machine = dummy_machine();
        let es = zvm_es(&machine);
        let meta = empty_meta();
        let history: Vec<std::sync::Arc<TurnRecord>> = vec![
            std::sync::Arc::new(TurnRecord {
                turn: 1, command: "one".into(), save: save1.clone(),
                map_snapshot: None, transcript: "First turn text.".into(),
            }),
            std::sync::Arc::new(TurnRecord {
                turn: 2, command: "two".into(), save: save2.clone(),
                map_snapshot: None, transcript: "Second turn text.".into(),
            }),
        ];
        let path = temp_archive_path("reuse-rewind-replay");
        let session = SessionRecord { history: &history, ..SessionRecord::empty() };

        // First write establishes the previous archive; second write reuses
        // both turns (asserted, so this test also falsifies as intended).
        let bytes1 = build_archive_bytes(
            &mapper, &es, Some(&machine.screen), &machine.aux_data, &meta, &session,
            &[], None, None, None, Some(&path), None,
        ).expect("first build");
        crate::storage::atomic_write(&path, &bytes1).expect("first write");

        let mut stats2 = HistoryReuseStats::default();
        let bytes2 = build_archive_bytes(
            &mapper, &es, Some(&machine.screen), &machine.aux_data, &meta, &session,
            &[], None, None, None, Some(&path), Some(&mut stats2),
        ).expect("second build");
        assert_eq!(stats2, HistoryReuseStats { raw_copied: 2, encoded: 0 }, "both turns raw-copied on the second write");
        crate::storage::atomic_write(&path, &bytes2).expect("second write");

        let ac = load_archive(&path).expect("load raw-copied archive");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.history.len(), 2);
        assert_eq!(ac.history[0].save, save1, "raw-copied turn 1 is byte-identical to the original snapshot");
        assert_eq!(ac.history[1].save, save2, "raw-copied turn 2 is byte-identical to the original snapshot");

        // Rewind: restore turn 1's raw-copied save into a fresh Machine.
        let plan1 = crate::history::resume_plan(&ac.history, 0);
        assert_eq!(plan1.turn, 1);
        let mem1 = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut rewound = zvm::cpu::exec::Machine::new(mem1);
        rewound.init_caps();
        rewound.restore_quetzal(&plan1.save).expect("rewound turn's raw-copied Quetzal restores");

        // Replay: restore turn 2's raw-copied save (the post-turn snapshot
        // "replaying" the second turn lands on).
        let plan2 = crate::history::resume_plan(&ac.history, 1);
        assert_eq!(plan2.turn, 2);
        let mem2 = zvm::memory::Memory::new(story).unwrap();
        let mut replayed = zvm::cpu::exec::Machine::new(mem2);
        replayed.init_caps();
        replayed.restore_quetzal(&plan2.save).expect("replayed turn's raw-copied Quetzal restores");
    }

    // -------------------------------------------------------------------------
    // round-trip: map JSON and save bytes survive a write-read cycle
    // -------------------------------------------------------------------------
    #[test]
    fn round_trip_map_and_save_bytes() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else {
            return; // skip if fixture absent
        };

        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 {
            let _ = machine.step();
        }

        let mapper = small_mapper();
        let expected_map_json = to_json(&mapper);
        let expected_save = machine.save_quetzal();

        let path = temp_archive_path("roundtrip");
        save_archive_m(&path, &mapper, &machine, &[], &[], &[], &[], &[]).expect("save_archive");

        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        // Map round-trips via JSON comparison (same as persist_files tests)
        assert_eq!(to_json(&ac.mapper), expected_map_json, "map JSON must match");

        // Save bytes are byte-identical
        assert_eq!(ac.save, expected_save, "save bytes must be identical");

        // Meta
        assert_eq!(ac.meta.format_version, CURRENT_FORMAT_VERSION);
        assert!(ac.meta.ifid.is_none());
    }

    /// SQ-0439: a seam the player declined must come back out of the archive still declined.
    ///
    /// These are player DECISIONS, so nothing downstream can recompute them — and a Restore State
    /// that quietly re-armed every prompt would re-ask about the trapdoor the player already said
    /// no to, which is exactly the nagging the memory exists to prevent.
    ///
    /// The restore itself is not the test. The CROSSING after it is: everything looks fine at the
    /// moment a map is loaded, and the seam only has anything to say the next time it is walked.
    ///
    /// Both ends of the trapdoor are exercised (SQ-0853), because a region can now be noticed on
    /// the way IN as well as on the way out: `3 -Up-> 1` is the climb, `1 -Down-> 3` is the descent,
    /// and an archive that carried only one of them would re-ask about the other.
    #[test]
    fn a_declined_layer_suggestion_survives_the_archive() {
        use mapper::layer::{move_region, planar_region, MoveTarget};
        use mapper::suggest::{SeamDecision, SeamKey};

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        // A four-room cellar behind one trapdoor, walked back to the foot of the stairs.
        let mut mapper = Mapper::default();
        mapper.observe(1, "Hall", None);
        mapper.observe(2, "Study", Some(Direction::E));
        mapper.observe(1, "Hall", Some(Direction::W));
        mapper.observe(3, "Cellar", Some(Direction::Down));
        mapper.observe(4, "Wine Cellar", Some(Direction::E));
        mapper.observe(5, "Vault", Some(Direction::E));
        mapper.observe(6, "Crypt", Some(Direction::E));
        mapper.observe(5, "Vault", Some(Direction::W));
        mapper.observe(4, "Wine Cellar", Some(Direction::W));
        mapper.observe(3, "Cellar", Some(Direction::W));
        let seam = SeamKey { from: 3, dir: Direction::Up };
        let descent = SeamKey { from: 1, dir: Direction::Down };
        mapper.graph.set_seam_decision(seam, SeamDecision::Ignored);
        mapper.graph.set_seam_decision(descent, SeamDecision::Ignored);

        let path = temp_archive_path("seam-memory");
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &[], &[])
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        let mut restored = ac.mapper;
        assert_eq!(restored.graph.seam_decision(seam), SeamDecision::Ignored);
        assert_eq!(restored.graph.seam_decision(descent), SeamDecision::Ignored);

        // Perturb, THEN assert: climb the stairs the player already declined to split, and walk
        // back down them, which is the other moment the same region can be noticed at.
        restored.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(
            restored.take_suggestion(),
            None,
            "a restored game does not re-ask about a seam already declined"
        );
        restored.observe(3, "Cellar", Some(Direction::Down));
        assert_eq!(
            restored.take_suggestion(),
            None,
            "…and no more so on the way back in than on the way out"
        );

        // ...and the same walk on an otherwise identical map that never declined it does ask, so
        // the silence above is the memory and not the fixture.
        let mut fresh = mapper::persist::from_json(&to_json(&small_cellar_mapper())).unwrap();
        fresh.observe(1, "Hall", Some(Direction::Up));
        let s = fresh.take_suggestion().expect("an un-declined seam still speaks up");
        assert_eq!(s.seam, seam);
        assert_eq!(s.region.rooms, [3, 4, 5, 6].into_iter().collect());
        assert_eq!(s.destinations, vec![MoveTarget::New]);
        fresh.observe(3, "Cellar", Some(Direction::Down));
        let down = fresh.take_suggestion().expect("and so does the descent, keyed on the trapdoor");
        assert_eq!(down.seam, descent);
        assert_eq!(down.region.rooms, [3, 4, 5, 6].into_iter().collect());
        // And the region it names is the one an accepted move would take.
        let mut g = fresh.graph.clone();
        let region = planar_region(&g, 3);
        assert_eq!(region.rooms, s.region.rooms);
        move_region(&mut g, &region, MoveTarget::New).expect("the suggestion is actionable");
    }

    /// SQ-0439, the same guarantee one layer up: a "Never" pressed IN THE PROMPT must come back
    /// out of the archive still meaning never.
    ///
    /// The test above pins the mapper's own memory; this one pins the path a player actually
    /// takes — the prompt writes the decision, the archive carries it, and the next crossing has
    /// to stay quiet. As before, the restore is not the test: the CROSSING after it is.
    #[test]
    fn a_never_pressed_in_the_prompt_survives_the_archive() {
        use crate::input::{apply_region_prompt, offer_layer_suggestion};
        use crate::state::{AppState, RegionPromptAct};
        use mapper::suggest::{SeamDecision, SeamKey};

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        let mut state = AppState::default();
        let mut mapper = small_cellar_mapper();
        mapper.observe(1, "Hall", Some(Direction::Up)); // the return crossing
        offer_layer_suggestion(&mut state, &mut mapper);
        assert!(state.overlays.region_prompt.is_some(), "the prompt is what is being answered");
        apply_region_prompt(&mut state, &mut mapper, RegionPromptAct::Never);

        // The same region is also noticed on the way IN, and that is a different passage with its
        // own answer (SQ-0853). Press Never on that one too, so the archive has to carry both.
        mapper.observe(3, "Cellar", Some(Direction::Down));
        offer_layer_suggestion(&mut state, &mut mapper);
        assert!(state.overlays.region_prompt.is_some(), "the descent asks its own question");
        apply_region_prompt(&mut state, &mut mapper, RegionPromptAct::Never);

        let seam = SeamKey { from: 3, dir: Direction::Up };
        let descent = SeamKey { from: 1, dir: Direction::Down };
        let path = temp_archive_path("prompt-never");
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &[], &[])
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        let mut restored = ac.mapper;
        assert_eq!(restored.graph.seam_decision(seam), SeamDecision::Ignored);
        assert_eq!(restored.graph.seam_decision(descent), SeamDecision::Ignored);

        // Perturb, THEN assert: walk back down and climb out again.
        let mut after = AppState::default();
        restored.observe(3, "Cellar", Some(Direction::Down));
        restored.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut after, &mut restored);
        assert!(
            after.overlays.region_prompt.is_none(),
            "a restored game does not re-ask about a passage the player answered 'never' to"
        );

        // …and the identical walk on a map that never answered DOES ask, so the silence above is
        // the memory and not the fixture.
        let mut fresh = small_cellar_mapper();
        let mut control = AppState::default();
        fresh.observe(1, "Hall", Some(Direction::Up));
        offer_layer_suggestion(&mut control, &mut fresh);
        assert!(control.overlays.region_prompt.is_some(), "an unanswered seam still speaks up");
    }

    /// The fixture `a_declined_layer_suggestion_survives_the_archive` compares against: the same
    /// manor, with nothing declined.
    fn small_cellar_mapper() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Study", Some(Direction::E));
        m.observe(1, "Hall", Some(Direction::W));
        m.observe(3, "Cellar", Some(Direction::Down));
        m.observe(4, "Wine Cellar", Some(Direction::E));
        m.observe(5, "Vault", Some(Direction::E));
        m.observe(6, "Crypt", Some(Direction::E));
        m.observe(5, "Vault", Some(Direction::W));
        m.observe(4, "Wine Cellar", Some(Direction::W));
        m.observe(3, "Cellar", Some(Direction::W));
        m
    }

    // -------------------------------------------------------------------------
    // read_archive_meta: cheap meta-only read matches load_archive's meta
    // -------------------------------------------------------------------------
    #[test]
    fn read_archive_meta_matches_load_archive_meta() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        let path = temp_archive_path("meta-only");
        let machine = dummy_machine();
        save_archive_meta(
            &path,
            &small_mapper(),
            &zvm_es(&machine),
            Some(&machine.screen),
            &machine.aux_data,
            Meta {
                format_version: CURRENT_FORMAT_VERSION,
                ifid: Some("ZCODE-1-000000-0000".to_string()),
                name: Some("before-troll".to_string()),
                turns: 42,
                saved_at: "2026-06-30T12:00:00Z".to_string(),
            location: None,
            score: None,
            trigger: SaveTrigger::HostState,
            },
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("save_archive_meta");

        let full = load_archive(&path).unwrap().meta;
        let quick = read_archive_meta(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(quick.format_version, full.format_version);
        assert_eq!(quick.ifid, full.ifid);
        assert_eq!(quick.name, full.name);
        assert_eq!(quick.turns, full.turns);
        assert_eq!(quick.saved_at, full.saved_at);
    }

    // -------------------------------------------------------------------------
    // corrupt ZIP -> Err, not a panic
    // -------------------------------------------------------------------------
    #[test]
    fn corrupt_zip_returns_err() {
        let path = temp_archive_path("corrupt");
        std::fs::write(&path, b"this is not a zip file").unwrap();
        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "corrupt archive must return Err");
    }

    // -------------------------------------------------------------------------
    // valid ZIP but missing a required entry -> Err
    // -------------------------------------------------------------------------
    #[test]
    fn missing_entry_returns_err() {
        use std::io::Write as _;

        let path = temp_archive_path("missing-entry");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write only meta.json; omit map.json and game.sav
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "archive missing map.json must return Err");
    }

    // -------------------------------------------------------------------------
    // back-compat: old archive (no name/turns/saved_at fields) still loads
    // -------------------------------------------------------------------------
    #[test]
    fn old_archive_without_new_meta_fields_loads_with_defaults() {
        use std::io::Write as _;

        let path = temp_archive_path("backcompat");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write a meta.json with only the original two fields.
            let old_meta_json = r#"{"format_version":1,"ifid":"ZCODE-1-000000-0000"}"#;
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(old_meta_json.as_bytes()).unwrap();

            // map.json: minimal valid mapper JSON
            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            // game.sav: empty bytes (won't be restored in this test)
            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&[]).unwrap();

            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("old archive should load");
        let _ = std::fs::remove_file(&path);

        assert!(ac.meta.name.is_none(), "name defaults to None");
        assert_eq!(ac.meta.turns, 0, "turns defaults to 0");
        assert_eq!(ac.meta.saved_at, "", "saved_at defaults to empty string");
        assert_eq!(ac.meta.ifid.as_deref(), Some("ZCODE-1-000000-0000"));
    }

    // -------------------------------------------------------------------------
    // transcript round-trip: lines + kinds survive write-read cycle
    // -------------------------------------------------------------------------
    #[test]
    fn transcript_round_trip() {
        use crate::state::TranscriptKind;
        use std::io::Write as _;

        let path = temp_archive_path("transcript-rt");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            // meta.json
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();

            // map.json
            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            // game.sav
            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&[]).unwrap();

            // transcript.json with mixed Story/Meta entries
            let td = TranscriptData {
                lines: vec!["West of House".to_string(), "/help".to_string(), "You are standing...".to_string()],
                kinds: vec![TranscriptKind::Story, TranscriptKind::Meta, TranscriptKind::Story],
                runs: Vec::new(),
                para: Vec::new(),
                images: Vec::new(),
            };
            let transcript_json = serde_json::to_string(&td).unwrap();
            zip.start_file(ENTRY_TRANSCRIPT, options).unwrap();
            zip.write_all(transcript_json.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        assert_eq!(ac.transcript, vec!["West of House", "/help", "You are standing..."]);
        assert_eq!(ac.transcript_kinds, vec![TranscriptKind::Story, TranscriptKind::Meta, TranscriptKind::Story]);
        assert_eq!(ac.transcript.len(), ac.transcript_kinds.len(), "vecs must be equal length");
    }

    #[test]
    fn transcript_data_round_trips_runs() {
        use crate::state::{StyleRun, TranscriptKind};
        let td = TranscriptData {
            lines: vec!["a".into(), "b".into()],
            kinds: vec![TranscriptKind::Story, TranscriptKind::Input],
            runs: vec![vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }], vec![]],
            para: Vec::new(),
            images: Vec::new(),
        };
        let json = serde_json::to_string(&td).unwrap();
        let back: TranscriptData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.runs, td.runs);
    }

    /// SQ-0538: the transcript archive stores LOGICAL lines and re-wraps them at
    /// render, so the `buffer_mode` state must survive with them — it rides along
    /// in `ParaFmt::nowrap_from`, and an archive written before the field existed
    /// loads as `None` (fully buffered).
    #[test]
    fn transcript_para_round_trips_buffer_mode_nowrap() {
        use crate::state::{ParaFmt, TranscriptKind};
        let td = TranscriptData {
            lines: vec!["Please wait....".into()],
            kinds: vec![TranscriptKind::Story],
            runs: Vec::new(),
            para: vec![ParaFmt { nowrap_from: Some(11), ..ParaFmt::default() }],
            images: Vec::new(),
        };
        let json = serde_json::to_string(&td).unwrap();
        let back: TranscriptData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.para, td.para);
        let old: ParaFmt = serde_json::from_str(r#"{"indent":0,"para_indent":0,"justify":0}"#).unwrap();
        assert_eq!(old.nowrap_from, None, "pre-SQ-0538 archives load as buffered");
    }

    #[test]
    fn old_transcript_json_loads_with_empty_runs() {
        // JSON without a "runs" field (older archive)
        let json = r#"{"lines":["x"],"kinds":["Story"]}"#;
        let td: TranscriptData = serde_json::from_str(json).unwrap();
        assert!(td.runs.is_empty());
    }

    // -------------------------------------------------------------------------
    // missing transcript entry -> empty vecs (graceful default for old archives)
    // -------------------------------------------------------------------------
    #[test]
    fn missing_transcript_defaults_to_empty() {
        use std::io::Write as _;

        let path = temp_archive_path("transcript-missing");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write an archive with no transcript.json entry.
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();

            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&[]).unwrap();

            // No ENTRY_TRANSCRIPT written.
            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("archive without transcript must load");
        let _ = std::fs::remove_file(&path);

        assert!(ac.transcript.is_empty(), "transcript must default to empty");
        assert!(ac.transcript_kinds.is_empty(), "transcript_kinds must default to empty");
    }

    // -------------------------------------------------------------------------
    // format freeze (docs/release/save-format-policy.md): the .lanthorn archive
    // version is frozen at 5 (bumped from 4 by SQ-0531, which added `Meta.trigger`
    // so an archive says whether the game bytes inside it are the game's own
    // `@save` or a host snapshot). Changing this constant must be a deliberate
    // bump (update this pin + a migration/release note), never accidental drift.
    #[test]
    fn format_version_constant_is_frozen() {
        assert_eq!(CURRENT_FORMAT_VERSION, 8, "archive format_version changed — see docs/release/save-format-policy.md");
    }

    // SQ-0531: `trigger` is persisted metadata, so its wire spelling is pinned —
    // the strings go on a user's disk and a rename would silently reclassify
    // every archive as a host snapshot (the serde default).
    #[test]
    fn save_trigger_wire_names_are_pinned_and_round_trip() {
        let ingame = serde_json::to_string(&SaveTrigger::Ingame).unwrap();
        let host = serde_json::to_string(&SaveTrigger::HostState).unwrap();
        assert_eq!(ingame, "\"ingame\"");
        assert_eq!(host, "\"hoststate\"");
        assert_eq!(serde_json::from_str::<SaveTrigger>(&ingame).unwrap(), SaveTrigger::Ingame);
        assert_eq!(serde_json::from_str::<SaveTrigger>(&host).unwrap(), SaveTrigger::HostState);
        // Only the game's own `@save` is interchange-grade (Quetzal §5.8).
        assert!(SaveTrigger::Ingame.is_portable());
        assert!(!SaveTrigger::HostState.is_portable());
        // A meta.json with no `trigger` at all reads as a host snapshot: the only
        // kind that existed before the field.
        let bare: Meta = serde_json::from_str(r#"{"format_version":5,"ifid":null}"#).unwrap();
        assert_eq!(bare.trigger, SaveTrigger::HostState);
    }

    // The trigger survives a real archive write/read, not just serde in memory.
    #[test]
    fn trigger_round_trips_through_a_written_archive() {
        let machine = dummy_machine();
        for trigger in [SaveTrigger::Ingame, SaveTrigger::HostState] {
            let path = temp_archive_path(if trigger.is_portable() { "trig-ingame" } else { "trig-host" });
            let meta = Meta {
                format_version: CURRENT_FORMAT_VERSION,
                ifid: None,
                name: None,
                turns: 0,
                saved_at: String::new(),
                location: None,
                score: None,
                trigger,
            };
            save_archive_meta(&path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen),
                &machine.aux_data, meta, &[], &[], &[], &[], &[], &[]).expect("write");
            assert_eq!(read_archive_meta(&path).unwrap().trigger, trigger, "read_archive_meta");
            assert_eq!(load_archive(&path).unwrap().meta.trigger, trigger, "load_archive");
            let _ = std::fs::remove_file(&path);
        }
    }

    // unknown format_version -> Err
    // -------------------------------------------------------------------------
    #[test]
    fn unknown_format_version_returns_err() {
        use std::io::Write as _;

        let path = temp_archive_path("bad-version");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            let meta = Meta { format_version: 99, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "unknown format_version must return Err");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("99"), "error should mention the bad version: {msg}");
    }

    #[test]
    fn archive_round_trips_aux_data() {
        let mut machine = dummy_machine();
        machine.aux_data.insert("hints".to_string(), vec![1, 2, 3]);
        let path = temp_archive_path("aux");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.aux.get("hints").map(|v| v.as_slice()), Some(&[1, 2, 3][..]));
    }

    /// Read a single ZIP entry's bytes, or `None` when the entry is absent.
    fn read_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
        let file = std::fs::File::open(path).ok()?;
        let mut zip = zip::ZipArchive::new(file).ok()?;
        let mut e = zip.by_name(name).ok()?;
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    #[test]
    fn engine_save_round_trips_through_archive() {
        // A zvm-tagged EngineSave writes game.sav == its bytes + engine.txt ==
        // "zmachine"; a Some(screen) writes screen.json; load returns the tag,
        // bytes, and screen.
        let machine = dummy_machine();
        let es = zvm_es(&machine);
        let path = temp_archive_path("engine-save-rt");
        save_archive(&path, &small_mapper(), &es, Some(&machine.screen), &machine.aux_data,
            &[], &[], &[], &[], &[], &[]).expect("save");

        assert_eq!(read_entry(&path, "game.qzl").as_deref(), Some(es.bytes.as_slice()),
            "game.qzl holds the EngineSave bytes");
        assert_eq!(read_entry(&path, ENTRY_ENGINE).as_deref(), Some(DEFAULT_ENGINE.as_bytes()),
            "engine.txt holds the engine tag");
        assert!(read_entry(&path, ENTRY_SCREEN).is_some(), "screen.json written for zvm");

        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE);
        assert_eq!(ac.save, es.bytes, "loaded save bytes match");
        assert!(ac.screen.is_some(), "screen restored");
        assert_eq!(ac.engine_save(), es, "engine_save() reconstructs the EngineSave");
    }

    #[test]
    fn glulx_tagged_save_round_trips_without_screen() {
        // A "glulx"-tagged save (no screen) writes NO screen.json; load reports
        // the glulx tag, the bytes, and screen == None.
        let es = EngineSave::new("glulx", 1, vec![9, 8, 7, 6]);
        let path = temp_archive_path("glulx-no-screen");
        save_archive(&path, &small_mapper(), &es, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[], &[]).expect("save");

        assert!(read_entry(&path, ENTRY_SCREEN).is_none(), "no screen.json for glulx");
        assert_eq!(read_entry(&path, ENTRY_ENGINE).as_deref(), Some(b"glulx".as_slice()));

        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, "glulx");
        assert_eq!(ac.save, vec![9, 8, 7, 6]);
        assert!(ac.screen.is_none(), "glulx archive carries no screen");
    }

    #[test]
    fn inner_save_entry_extension_matches_engine() {
        // The inner save entry is named for the engine's format: game.qzl for the
        // Z-machine, game.glksave for Glulx, and never the old game.sav.
        let zm = EngineSave::new(DEFAULT_ENGINE, 1, vec![1, 2, 3]);
        let zpath = temp_archive_path("entry-ext-zm");
        save_archive(&zpath, &small_mapper(), &zm, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[], &[]).expect("save zm");
        assert!(read_entry(&zpath, "game.qzl").is_some(), "Z-machine save entry is game.qzl");
        assert!(read_entry(&zpath, "game.sav").is_none(), "no legacy game.sav entry");
        assert!(read_entry(&zpath, "game.glksave").is_none());
        assert_eq!(load_archive(&zpath).unwrap().save, vec![1, 2, 3], "zm round-trips");
        let _ = std::fs::remove_file(&zpath);

        let gl = EngineSave::new("glulx", 1, vec![4, 5, 6]);
        let gpath = temp_archive_path("entry-ext-gl");
        save_archive(&gpath, &small_mapper(), &gl, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[], &[]).expect("save gl");
        assert!(read_entry(&gpath, "game.glksave").is_some(), "Glulx save entry is game.glksave");
        assert!(read_entry(&gpath, "game.qzl").is_none());
        assert!(read_entry(&gpath, "game.sav").is_none(), "no legacy game.sav entry");
        assert_eq!(load_archive(&gpath).unwrap().save, vec![4, 5, 6], "glulx round-trips");
        let _ = std::fs::remove_file(&gpath);
    }

    #[test]
    fn archive_without_engine_txt_loads_as_zmachine() {
        // An archive with NO engine.txt must default to the Z-machine: its
        // game.qzl bytes load as EngineSave { "zmachine", <bytes> } + the screen.
        let machine = dummy_machine();
        let quetzal = machine.save_quetzal();
        let path = temp_archive_path("old-no-engine");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            let meta = Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState };
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(serde_json::to_string(&meta).unwrap().as_bytes()).unwrap();
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(mapper::persist::to_json(&small_mapper()).as_bytes()).unwrap();
            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&quetzal).unwrap();
            // screen.json present, engine.txt absent (the old format).
            zip.start_file(ENTRY_SCREEN, options).unwrap();
            zip.write_all(serde_json::to_string(&ScreenDto::from_screen(&machine.screen)).unwrap().as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("old archive loads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE, "absent engine.txt defaults to zmachine");
        assert_eq!(ac.save, quetzal, "raw Quetzal bytes load unchanged");
        assert!(ac.screen.is_some(), "legacy screen.json still loads");
        assert_eq!(ac.engine_save().engine, DEFAULT_ENGINE);
        assert_eq!(ac.engine_save().bytes, quetzal);
    }

    #[test]
    fn v6_window_table_round_trips_through_screen_dto() {
        // A populated v6 8-window table (geometry, cursor, margins, colours, a
        // grid glyph and a pixel-text run) survives ScreenDto → JSON → ScreenDto
        // → ScreenState losslessly, so a host Save State reproduces v6 chrome.
        use zvm::screen::{Cell, ScreenState, V6Text, V6Windows, ZColour};
        let mut v6 = V6Windows::default();
        v6.current = 7;
        // Window 0: the main text window box + a colour pair.
        let w0 = &mut v6.windows[0];
        w0.put_prop(0, 40);  // y_coord
        w0.put_prop(1, 44);  // x_coord
        w0.put_prop(2, 160); // y_size
        w0.put_prop(3, 234); // x_size
        w0.put_prop(6, 8);   // left_margin
        w0.fg = ZColour::Standard(2);
        w0.bg = ZColour::True(0x1234);
        // Window 1: a status grid with one styled glyph + a pixel-text run.
        let w1 = &mut v6.windows[1];
        w1.put_prop(3, 320);
        w1.put_prop(2, 8);
        w1.grid.resize(1, 4);
        w1.grid.put(1, 2, 'Z', 0x01, ZColour::Standard(3), ZColour::Standard(9));
        w1.texts.push(V6Text::derived(6, 139, "SCORE".into(), 2, ZColour::True24(0xABCDEF), ZColour::Default, zvm::screen::V6Cell::DEFAULT));
        // …and the OTHER two pixel-run layers of the same window (SQ-0820): prose
        // the window has streamed, and prose a move left frozen behind it. Both are
        // live screen state nothing repaints after a restore.
        w1.streamed.push(V6Text::derived(247, 76, "Current Bet:".into(), 0, ZColour::Standard(4), ZColour::True(0x0421), zvm::screen::V6Cell::DEFAULT));
        w1.retired.push(V6Text::derived(49, 297, "SHOGUN".into(), 4, ZColour::Default, ZColour::Standard(9), zvm::screen::V6Cell::DEFAULT));

        let src = ScreenState { v6: Some(v6), ..Default::default() };
        let dto = ScreenDto::from_screen(&src);
        let json = serde_json::to_string(&dto).unwrap();
        let back: ScreenDto = serde_json::from_str(&json).unwrap();
        let out = back.to_screen();

        let rv = out.v6.expect("v6 table restored");
        assert_eq!(rv.current, 7);
        assert_eq!((rv.windows[0].y_coord, rv.windows[0].x_coord), (40, 44));
        assert_eq!((rv.windows[0].y_size, rv.windows[0].x_size), (160, 234));
        assert_eq!(rv.windows[0].left_margin, 8);
        assert_eq!(rv.windows[0].fg, ZColour::Standard(2));
        assert_eq!(rv.windows[0].bg, ZColour::True(0x1234));
        let c: Cell = rv.windows[1].grid.cell(1, 2);
        assert_eq!((c.ch, c.style, c.fg, c.bg), ('Z', 0x01, ZColour::Standard(3), ZColour::Standard(9)));
        assert_eq!(rv.windows[1].texts.len(), 1);
        assert_eq!(rv.windows[1].texts[0], V6Text::derived(6, 139, "SCORE".into(), 2, ZColour::True24(0xABCDEF), ZColour::Default, zvm::screen::V6Cell::DEFAULT));
        assert_eq!(
            rv.windows[1].streamed,
            vec![V6Text::derived(247, 76, "Current Bet:".into(), 0, ZColour::Standard(4), ZColour::True(0x0421), zvm::screen::V6Cell::DEFAULT)],
            "SQ-0820: a prose window's streamed runs are live screen state, so they ride in the archive beside `texts`"
        );
        assert_eq!(
            rv.windows[1].retired,
            vec![V6Text::derived(49, 297, "SHOGUN".into(), 4, ZColour::Default, ZColour::Standard(9), zvm::screen::V6Cell::DEFAULT)],
            "SQ-0820: and so does the prose a move or resize froze in place"
        );
    }

    #[test]
    fn non_v6_screen_dto_has_no_v6_table() {
        // A classic (v5) screen serializes v6 as None; to_screen restores None.
        let src = zvm::screen::ScreenState::default(); // v6 = None
        let dto = ScreenDto::from_screen(&src);
        let json = serde_json::to_string(&dto).unwrap();
        let back: ScreenDto = serde_json::from_str(&json).unwrap();
        assert!(back.to_screen().v6.is_none());
    }

    #[test]
    fn picture_blobs_round_trip_through_archive() {
        // Per-window graphics PNG blobs survive save_archive_meta_pics →
        // load_archive byte-for-byte, keyed and sorted by window number.
        let machine = dummy_machine();
        let png_a = vec![0x89, b'P', b'N', b'G', 1, 2, 3];   // opaque stand-in blobs;
        let png_b = vec![0x89, b'P', b'N', b'G', 9, 8, 7, 6]; // load doesn't decode them
        let path = temp_archive_path("pics");
        save_archive_meta_pics(
            &path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen), &machine.aux_data,
            Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState },
            &SessionRecord::empty(),
            &[(7, png_a.clone()), (1, png_b.clone())],
            None,
            None,
        ).expect("save with pictures");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        // Write order (paint order) is preserved, NOT re-sorted by window number.
        assert_eq!(ac.pictures, vec![(7, png_a), (1, png_b)], "blobs round-trip in write order");
    }

    #[test]
    fn archive_without_pictures_loads_empty() {
        let machine = dummy_machine();
        let path = temp_archive_path("nopics");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.pictures.is_empty(), "archive without pictures/ → empty pictures");
    }

    // ── SQ-0644: the archive is written atomically ───────────────────────────

    /// The auto-save rewrites `default.lanthorn` EVERY turn, and the quit path can be
    /// cut short by the exit watchdog — and it used to open the player's archive with
    /// `File::create`, truncating it before a byte of the replacement existed. A write
    /// that cannot complete must now leave the previous archive fully readable, which
    /// a directory that admits no new files proves: the temp sibling can't be created,
    /// so the save fails outright where the in-place write would have destroyed the
    /// old archive on its way to the new one.
    #[test]
    fn an_interrupted_archive_write_keeps_the_previous_archive() {
        let dir = std::env::temp_dir().join(format!("lanthorn-arch-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("default.lanthorn");
        let machine = dummy_machine();
        let meta = |turns: u32| Meta {
            format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns,
            saved_at: String::new(), location: None, score: None, trigger: SaveTrigger::HostState,
        };
        save_archive_meta(&path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen),
            &machine.aux_data, meta(1), &[], &[], &[], &[], &[], &[]).expect("first save");

        if !crate::storage::deny_new_files_in(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // platform can't enforce it (or we're root) — skip
        }
        let result = save_archive_meta(&path, &small_mapper(), &zvm_es(&machine), Some(&machine.screen),
            &machine.aux_data, meta(2), &[], &[], &[], &[], &[], &[]);
        crate::storage::allow_new_files_in(&dir);

        assert!(result.is_err(), "a save that cannot complete must fail, not half-happen");
        let ac = load_archive(&path).expect("the previous archive is still a loadable archive");
        assert_eq!(ac.meta.turns, 1, "and it is still the PREVIOUS save, not a stump of the new one");
        assert_eq!(ac.save, machine.save_quetzal(), "its game bytes are intact");
        assert!(crate::storage::leftover_temps(&dir).is_empty(), "no temp left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── SQ-0647: a restored screen is validated, not trusted ─────────────────

    /// `screen.json` is a file on the player's disk; nothing guarantees its `cells`
    /// vector matches its own `cols × rows`. zvm indexes the grid by those dimensions
    /// (`UpperWindow::resize_preserving`, and every read after it), so a short vector
    /// panicked the app on the first repaint AFTER the restore — never on the load,
    /// where it might have been diagnosed. Repair the grid on the way in instead.
    #[test]
    fn a_short_cell_vector_is_repaired_rather_than_left_to_panic() {
        let mut dto = ScreenDto::from_screen(&zvm::screen::ScreenState::default());
        dto.cols = 8;
        dto.rows = 4;
        dto.cells = vec![('x', 0); 3]; // truncated file: 3 cells for a 32-cell grid

        let mut scr = dto.to_screen();
        assert_eq!(
            scr.upper.cells.len(),
            scr.upper.cols as usize * scr.upper.rows as usize,
            "the grid invariant every consumer assumes",
        );
        assert_eq!(scr.upper.cell(1, 1).ch, 'x', "what the file DID hold is kept");
        // The first repaint after a restore: a resize to the live pane. Pre-fix this
        // is the panic (`cells[r * cols + c]` past the end of a 3-cell vector).
        scr.upper.resize_preserving(4, 6);
        assert_eq!(scr.upper.cells.len(), 24);
    }

    /// A too-LONG vector is the same defect from the other side, and absurd dimensions
    /// are a third: `65535 × 65535` would ask for a four-billion-cell allocation.
    /// Both are clamped to something a screen could actually be — a restore reconciles
    /// the saved screen against the current pane anyway.
    #[test]
    fn oversized_screen_dimensions_and_vectors_are_clamped() {
        let mut dto = ScreenDto::from_screen(&zvm::screen::ScreenState::default());
        dto.cols = 4;
        dto.rows = 2;
        dto.cells = vec![('y', 0); 500]; // far more cells than the grid claims
        let scr = dto.to_screen();
        assert_eq!(scr.upper.cells.len(), 8, "trimmed to cols × rows");

        let mut dto = ScreenDto::from_screen(&zvm::screen::ScreenState::default());
        dto.cols = u16::MAX;
        dto.rows = u16::MAX;
        dto.cells = Vec::new();
        let scr = dto.to_screen();
        assert!(scr.upper.cols <= MAX_GRID_COLS && scr.upper.rows <= MAX_GRID_ROWS, "clamped");
        assert_eq!(scr.upper.cells.len(), scr.upper.cols as usize * scr.upper.rows as usize);
    }

    /// ZMSD §8.4 has eight v6 windows and `windows[current]` is a fixed-array index
    /// that `Engine::screen()` performs on every frame — so an archived `current` of 9
    /// panicked on the frame after the restore, not on the load. Clamp it.
    #[test]
    fn an_out_of_range_current_v6_window_is_clamped() {
        let mut v6 = zvm::screen::V6Windows::default();
        v6.current = 3;
        let src = zvm::screen::ScreenState { v6: Some(v6), ..Default::default() };
        let mut dto = ScreenDto::from_screen(&src);
        dto.v6.as_mut().unwrap().current = 9; // hand-edited / corrupt archive

        let rv = dto.to_screen().v6.expect("v6 table restored");
        assert!((rv.current as usize) < rv.windows.len(), "current indexes a real window");
        let _ = &rv.windows[rv.current as usize]; // pre-fix: index out of bounds
    }

    /// End to end: an archive whose `screen.json` carries both defects still loads,
    /// and what it hands back is safe to drive. The loader's existing contract for a
    /// bad `screen.json` is tolerance (a corrupt one restores as "no saved screen" and
    /// the story repaints), so a merely INCONSISTENT one is repaired, not rejected —
    /// the text it holds is still the text that was on screen.
    #[test]
    fn an_archive_with_an_inconsistent_screen_json_loads_and_is_safe() {
        let machine = dummy_machine();
        let path = temp_archive_path("screen-corrupt");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");

        // Rewrite the archive with a screen.json whose grid and window table lie.
        let mut v6 = zvm::screen::V6Windows::default();
        v6.windows[1].grid.resize(2, 3);
        let src = zvm::screen::ScreenState { v6: Some(v6), ..Default::default() };
        let mut dto = ScreenDto::from_screen(&src);
        dto.cols = 20;
        dto.rows = 5;
        dto.cells = vec![('q', 0); 2];
        {
            let v6d = dto.v6.as_mut().unwrap();
            v6d.current = 200;
            v6d.windows[1].cols = 30; // window grid claims 30×9, carries 6 cells
            v6d.windows[1].rows = 9;
        }
        let bad_json = serde_json::to_string(&dto).unwrap();

        let rewritten = temp_archive_path("screen-corrupt-out");
        {
            let src_file = std::fs::File::open(&path).unwrap();
            let mut zin = zip::ZipArchive::new(src_file).unwrap();
            let out = std::fs::File::create(&rewritten).unwrap();
            let mut zout = zip::ZipWriter::new(out);
            for i in 0..zin.len() {
                let mut e = zin.by_index(i).unwrap();
                let name = e.name().to_string();
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                if name == ENTRY_SCREEN {
                    buf = bad_json.as_bytes().to_vec();
                }
                zout.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                zout.write_all(&buf).unwrap();
            }
            zout.finish().unwrap();
        }

        let ac = load_archive(&rewritten).expect("an inconsistent screen.json still loads");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&rewritten);

        let mut scr = ac.screen.expect("screen restored (repaired), not dropped");
        assert_eq!(scr.upper.cells.len(), scr.upper.cols as usize * scr.upper.rows as usize);
        let rv = scr.v6.as_ref().expect("v6 table");
        assert!((rv.current as usize) < rv.windows.len());
        for w in &rv.windows {
            assert_eq!(w.grid.cells.len(), w.grid.cols as usize * w.grid.rows as usize, "every v6 grid is consistent");
        }
        // Perturb exactly as the next frame would: index the current window and resize
        // the upper grid to the live pane. Pre-fix, either one panics.
        let _ = &rv.windows[rv.current as usize];
        scr.upper.resize_preserving(24, 80);
        assert_eq!(scr.upper.cells.len(), 24 * 80);
    }

    #[test]
    fn archive_without_aux_loads_empty_map() {
        let machine = dummy_machine(); // empty aux_data
        let path = temp_archive_path("noaux");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.aux.is_empty());
    }

    // ── PngBlobCache (SQ-1184) ──────────────────────────────────────────────

    fn solid_image(w: u32, h: u32, rgba: [u8; 4]) -> std::sync::Arc<image::RgbaImage> {
        let mut img = image::RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = image::Rgba(rgba);
        }
        std::sync::Arc::new(img)
    }

    #[test]
    fn png_blob_cache_reuses_bytes_for_the_same_arc() {
        let mut cache = PngBlobCache::default();
        let img = solid_image(4, 4, [10, 20, 30, 255]);

        let first = cache.encode(&img).expect("encode succeeds");
        let second = cache.encode(&img).expect("encode succeeds again");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "the same Arc<RgbaImage> must hand back the identical cached PNG Arc, not a fresh encode"
        );
    }

    #[test]
    fn png_blob_cache_encodes_separately_for_distinct_images() {
        let mut cache = PngBlobCache::default();
        let a = solid_image(4, 4, [10, 20, 30, 255]);
        let b = solid_image(4, 4, [200, 100, 50, 255]);

        let png_a = cache.encode(&a).expect("encode a");
        let png_b = cache.encode(&b).expect("encode b");
        assert!(!std::sync::Arc::ptr_eq(&png_a, &png_b), "distinct images cache distinct bytes");
        assert_ne!(*png_a, *png_b, "different pixels encode to different PNG bytes");
        assert_eq!(cache.cache.len(), 2);
    }

    #[test]
    fn png_blob_cache_retain_live_evicts_absent_keeps_present() {
        let mut cache = PngBlobCache::default();
        let a = solid_image(2, 2, [1, 2, 3, 255]);
        let b = solid_image(2, 2, [4, 5, 6, 255]);
        let key_a = std::sync::Arc::as_ptr(&a) as usize;
        let key_b = std::sync::Arc::as_ptr(&b) as usize;
        cache.encode(&a);
        cache.encode(&b);
        assert_eq!(cache.cache.len(), 2);

        // Only `a` is still "live" (present in the latest session snapshot).
        let live: std::collections::HashSet<usize> = [key_a].into_iter().collect();
        cache.retain_live(&live);

        assert_eq!(cache.cache.len(), 1, "b's entry is evicted");
        assert!(cache.cache.contains_key(&key_a), "a's entry survives");
        assert!(!cache.cache.contains_key(&key_b));
    }
}
