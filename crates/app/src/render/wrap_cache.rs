//! The ONE owner of "has the transcript wrap moved, and by how much?" (SQ-1034).
//!
//! Both render paths wrap the whole scrollback and then show forty rows of it.
//! Before this module they disagreed about when that was necessary, in opposite
//! directions and for unrelated reasons:
//!
//! * the CELL path (which hybrid's story text also uses) had a whole-product
//!   cache keyed on `transcript_gen`, and `transcript_gen` moves on every
//!   mutation — so every turn threw the wrap away and rebuilt it from line zero.
//!   Its hit path idled flat at 1.7 ms from 100 turns to 20,000 while its cold
//!   render went 2.0 ms to 17.9 ms, which is the miss path hiding behind the hit
//!   path;
//! * the RASTER path had no cache at all, behind a whole-canvas gate that hashes
//!   `AppState::input.value` — so one keystroke on an unchanged transcript
//!   re-wrapped 20,000 turns, measured at 25.058 ms.
//!
//! The two are one mechanism. Content only ever GROWS at the end (a turn prints);
//! everything else — a resize, a filter, a theme, a screen-clear anchor moving —
//! changes where lines BREAK and is a rebuild. So the question every frame asks
//! is [`WrapKey::plan`]: reuse, append, or rebuild.
//!
//! ## Why one owner and not one cache
//!
//! The two products are genuinely different types — `WrappedRow`s carrying kinds,
//! styles, runs and image bands against the raster's glyph rows and emphasis bits
//! — so there are two cache structs. What must not be duplicated is the RULE, and
//! the rule is this file: one key type, one plan. Copying the cell path's cache
//! into the raster path would reproduce exactly the divergence that made this
//! quest exist, with a second copy to keep in step. CLAUDE.md's refactoring
//! policy names the shape: a hand-maintained invariant across files is the
//! symptom, and the cure is a type.
//!
//! ## Raster is the degenerate case, not a second design
//!
//! Raster's wrap width comes from `story_prose_box` over the NATIVE v6 screen
//! rect — the game's own coordinate space — so it is constant for the session and
//! a resize only rescales the finished composite. Raster therefore takes the
//! append branch essentially always. The cell path wraps to the TERMINAL's
//! columns and takes the rebuild branch on resize. Same mechanism, different
//! field volatility.
//!
//! ## What the key may and may not contain
//!
//! Every field here changes where a line breaks or what colour it breaks in.
//! `AppState::input.value` is the live counter-example: the input line is not
//! part of the wrapped product at all — `build_main_text` takes it as a separate
//! argument — so a keystroke has no business invalidating anything here, and does
//! not. Nor do the search query, the command-bar mode, or the viewport HEIGHT,
//! all of which act at windowing time on an already-wrapped product.

use std::hash::{Hash, Hasher};

use crate::state::{AppState, TranscriptFilter};

// ── The key ───────────────────────────────────────────────────────────────────

/// The layout facts — everything that moves a wrap BOUNDARY or the ink a row is
/// resolved in, minus the two that cannot be `Copy` (those are [`WrapInk`]).
///
/// One `Copy` value with one constructor, so a caller cannot assemble a subset
/// and get a plausible wrong answer: [`WrapShape::of`] is the only way to build
/// one from a live state, and adding the next fact is a field here rather than an
/// argument somewhere. (CLAUDE.md's refactoring policy; SQ-1034.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WrapShape {
    /// Wrap width. Body columns on the cell path; the native prose box's columns
    /// on the raster path, where it is constant for the session.
    pub width: u16,
    /// Which kinds are visible, hence which lines are wrapped at all.
    pub filter: TranscriptFilter,
    /// Whether inline-image bands are emitted (a game picker is present).
    pub images_enabled: bool,
    /// Picker cell pixel size — drives an image band's cell footprint.
    pub char_px: (u16, u16),
    /// Screen-clear anchor (a full-transcript index). Also the TRUNCATION case:
    /// `transcript.truncate` at a clear anchor or a rewind moves it, and this
    /// rebuilds rather than needing a path of its own.
    pub clear_anchor: Option<usize>,
    /// The MACHINE's own screen pair (`AppState::v6_page_pair`), the base a Story
    /// line's inherited channels resolve from under ZMSD §8.3 (SQ-0822). It can
    /// move without the transcript changing.
    pub machine_pair: Option<(u32, u32)>,
    /// The STORY WINDOW's own page (`AppState::v6_story_page`), which the period
    /// look's grounds are re-based onto (SQ-0954). Same reason as above.
    pub story_page: Option<(u8, u8, u8)>,
    /// The period look those grounds are re-based FROM.
    pub period_look: Option<zvm::interpreter::PeriodLook>,
    /// Whether hybrid's chrome ring is up, which decides an inline picture's
    /// scaled cell footprint (SQ-1002).
    pub hybrid_ring: bool,
    /// The pen, as [`crate::native_font::TextFace::wrap_fingerprint`] digests it.
    /// The raster wrap breaks by PIXEL (SQ-1009), so the face is a wrap input and
    /// not merely a drawing one.
    pub face: u64,
}

impl WrapShape {
    /// Is `self` identical to `other` in every field EXCEPT `clear_anchor`?
    ///
    /// Split out because a moved anchor is not always a rebuild (SQ-1179): it
    /// only ever changes top-anchoring/windowing, never where a line breaks,
    /// so [`WrapKey::plan`] gives it its own — narrower — safety check instead
    /// of folding it into a blanket shape comparison.
    fn eq_ignoring_anchor(&self, other: &WrapShape) -> bool {
        WrapShape { clear_anchor: other.clear_anchor, ..*self } == *other
    }

    /// Gather every layout fact from `state`. `width` is the caller's because it
    /// is the one fact the two paths derive differently — terminal columns
    /// against the native prose box.
    pub(crate) fn of(state: &AppState, width: u16) -> WrapShape {
        let char_px = state
            .game_picker
            .as_ref()
            .map(|p| {
                let f = p.font_size();
                (f.width, f.height)
            })
            .unwrap_or((1, 1));
        WrapShape {
            width,
            filter: state.transcript_filter,
            images_enabled: state.game_picker.is_some(),
            char_px,
            clear_anchor: state.clear_anchor,
            machine_pair: state.v6_page_pair.get(),
            story_page: state.v6_story_page.get(),
            period_look: state.period_look,
            hybrid_ring: state.v6_hybrid_ring.get(),
            face: state.v6_text.wrap_fingerprint(),
        }
    }
}

/// The two wrap inputs that are not `Copy`: the room name whole-line style rules
/// match on, and the resolved colour scheme they resolve through.
///
/// Held apart from [`WrapShape`] so the per-frame question can be asked without
/// cloning either — [`WrapInk::matches`] compares against the live state, and the
/// clone happens only where a rebuild is already being paid for.
#[derive(Debug, Clone)]
pub(crate) struct WrapInk {
    pub room_name: Option<String>,
    pub colors: crate::colors::ColorScheme,
}

impl WrapInk {
    pub(crate) fn of(state: &AppState) -> WrapInk {
        WrapInk { room_name: state.current_room_name.clone(), colors: state.colors.clone() }
    }

    /// Is this still what `state` would resolve rows through?
    pub(crate) fn matches(&self, state: &AppState) -> bool {
        self.room_name.as_deref() == state.current_room_name.as_deref() && self.colors == state.colors
    }
}

/// What the transcript itself looked like, as far as an already-wrapped prefix is
/// concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WrapContent {
    /// `AppState::transcript_edits` — mutations that were NOT pure appends. Any
    /// movement here means something already wrapped has changed underneath.
    pub edits: u64,
    /// Source lines consumed into the wrapped product.
    pub len: usize,
    /// A digest of the LAST consumed source line — its text, kind, style, runs,
    /// paragraph format and image.
    ///
    /// This is the guard, not the mechanism (CLAUDE.md: a guard beats a
    /// convention). `edits` is stated by hand at each mutator, and the next
    /// mutator is written by someone with no reason to know that. Every in-place
    /// mutator we have touches the last line — an echo appended to the prompt, a
    /// self-echo merged into it, an insert above it, a colour fill on it — so a
    /// misclassification shows up here as a changed fingerprint and rebuilds
    /// instead of appending onto a line that moved.
    pub tail: u64,
}

impl WrapContent {
    /// Digest `state`'s transcript as consumed up to `upto` source lines.
    ///
    /// `upto` is the CACHE's length, not the transcript's: the question being
    /// asked is "is the prefix I already wrapped still the same prefix?", so the
    /// fingerprint has to be taken at the same place both times.
    pub(crate) fn of(state: &AppState, upto: usize) -> WrapContent {
        let upto = upto.min(state.transcript.len());
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(i) = upto.checked_sub(1) {
            state.transcript[i].hash(&mut h);
            state.transcript_kinds.get(i).map(|k| *k as u8).hash(&mut h);
            // `Style` is not `Hash`; its debug form is stable and this runs once
            // per frame on one line.
            state.transcript_styles.get(i).map(|s| format!("{s:?}")).hash(&mut h);
            state.transcript_runs.get(i).hash(&mut h);
            state.transcript_para.get(i).hash(&mut h);
            state
                .transcript_images
                .get(i)
                .and_then(|im| im.as_ref())
                .map(|im| std::sync::Arc::as_ptr(&im.pixels) as usize)
                .hash(&mut h);
        }
        WrapContent { edits: state.transcript_edits, len: upto, tail: h.finish() }
    }
}

/// Everything a wrapped product was built for: its layout, its ink, and the
/// transcript prefix it consumed.
#[derive(Debug, Clone)]
pub(crate) struct WrapKey {
    pub shape: WrapShape,
    pub ink: WrapInk,
    pub content: WrapContent,
}

impl WrapKey {
    /// Gather every fact, for a product that consumed the whole transcript.
    pub(crate) fn of(state: &AppState, width: u16) -> WrapKey {
        WrapKey {
            shape: WrapShape::of(state, width),
            ink: WrapInk::of(state),
            content: WrapContent::of(state, state.transcript.len()),
        }
    }

    /// The work this frame owes against a product built for `self`.
    ///
    /// Cheapest test first, and deliberately: [`WrapShape`] is a `Copy` compare,
    /// [`WrapInk::matches`] walks a theme map, and neither clones. The hot path is
    /// a frame where nothing moved at all.
    pub(crate) fn plan(&self, state: &AppState, width: u16) -> WrapPlan {
        let cur_shape = WrapShape::of(state, width);
        if !self.shape.eq_ignoring_anchor(&cur_shape) {
            return WrapPlan::Rebuild;
        }
        // A moved screen-clear anchor is not, by itself, a rewrap (SQ-1179): it
        // only ever moves top-anchoring/windowing, never where a line breaks.
        // Safe whenever the new anchor sits at or after this cache's own
        // synced length — exactly what `mark_screen_clear` always sets it to
        // — because every already-cached filtered line then unconditionally
        // precedes it, with no need to inspect which. An anchor moved to
        // somewhere EARLIER (the truncation-adjacent case `WrapShape`'s own
        // doc names) is not provably safe this way and rebuilds instead.
        let anchor_moved = self.shape.clear_anchor != cur_shape.clear_anchor;
        if anchor_moved {
            let safe = match cur_shape.clear_anchor {
                None => true,
                Some(a) => a >= self.content.len,
            };
            if !safe {
                return WrapPlan::Rebuild;
            }
        }
        if !self.ink.matches(state) {
            return WrapPlan::Rebuild;
        }
        // (A) SQ-1179: an unbroken run of tail-inserts since this cache was
        // synced can be REPAIRED — re-wrap only the disturbed tail — instead
        // of rebuilt from line zero. Checked before the generic content
        // fingerprint below, which a genuine insert deliberately fails: the
        // fingerprinted position (`content.len - 1`) now holds different
        // text, because the line that used to be there moved.
        if self.content.len > 0 {
            if let Some(run) = state.transcript_tail_insert.get() {
                let at = self.content.len - 1;
                if run.since_edits == self.content.edits && run.min_at >= at && state.transcript.len() > self.content.len {
                    return WrapPlan::Repair { at };
                }
            }
        }
        if self.content != WrapContent::of(state, self.content.len) {
            return WrapPlan::Rebuild;
        }
        match state.transcript.len().cmp(&self.content.len) {
            // Unreachable while every shrink is a `TranscriptEdit::Rewrote` (and
            // `edits` is compared above) — but a shrink is the one miss that would
            // index past the end of the wrapped rows, so it is stated rather than
            // assumed.
            std::cmp::Ordering::Less => WrapPlan::Rebuild,
            std::cmp::Ordering::Equal if !anchor_moved => WrapPlan::Reuse,
            std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => WrapPlan::Append { from: self.content.len },
        }
    }
}

/// What a frame owes the wrap cache. See [`WrapKey::plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WrapPlan {
    /// Nothing moved: window the cached rows and draw.
    Reuse,
    /// Content grew (and/or the screen-clear anchor moved to a position that
    /// cannot disturb what is already cached) and nothing already wrapped
    /// moved: wrap source lines from `from` onwards and extend the product.
    Append { from: usize },
    /// Exactly an unbroken run of tail-inserts since this cache was synced
    /// (SQ-1179): every line before `at` provably did not move, so only the
    /// tail from `at` — the old cached tail line's raw index — needs
    /// re-wrapping, not the whole product.
    Repair { at: usize },
    /// A layout fact moved (or something already wrapped changed): throw the
    /// product away and wrap from line zero.
    Rebuild,
}

// ── The cell path's product ───────────────────────────────────────────────────

/// The cached wrapped-transcript product for the CELL path (and hybrid's story
/// text), for one [`WrapKey`].
pub(crate) struct CellWrapCache {
    /// The key these products were built for.
    pub key: WrapKey,
    /// Fully wrapped rows for the whole filtered transcript (oldest-first),
    /// INCLUDING any trailing float flush — this is what the draw path windows.
    pub rows: Vec<crate::render::transcript::WrappedRow>,
    /// Wrapped row each FILTERED source line starts at, for [`anchor_row`].
    ///
    /// [`crate::render::transcript::anchor_row_at`]'s answer moves as lines are
    /// appended even though the anchor itself has not — an anchor sitting exactly
    /// at the end is an empty post-clear screen, and the next line printed gives
    /// it a real row — so the index is kept and the anchor recomputed from it.
    pub starts: Vec<usize>,
    /// How many of `rows` were produced by source lines, before the trailing
    /// [`crate::render::transcript::flush_float`].
    ///
    /// A float whose picture outran its text flushes its remaining strips as
    /// empty rows at the END of the wrap. Those rows are not final: the next
    /// prose line to arrive rides BESIDE the picture and takes those strips over.
    /// So an append truncates back to here first and re-flushes afterwards.
    pub stable_rows: usize,
    /// The float still open after the last consumed line — the wrap's carry
    /// state, without which an appended line cannot know it should be narrowed.
    pub carry: Option<crate::render::transcript::FloatState>,
    /// The float carry ENTERING the last consumed line — i.e. `carry` one line
    /// earlier (SQ-1179). A repair that discards that last line (because an
    /// insert landed before it) needs to resume wrapping from exactly the carry
    /// state a fresh rebuild would have had there, which `carry` itself cannot
    /// supply once it has already been advanced past that line. Maintained by
    /// [`crate::render::transcript::wrap_lines_kinded_extend`]'s `pretail`
    /// out-param on every extend, cheaply (a clone of a small `Option`, almost
    /// always `None`) — so it is always current for whichever line is last.
    pub tail_entry_carry: Option<crate::render::transcript::FloatState>,
    /// Whether the raw source line at `key.content.len - 1` — the cache's own
    /// last consumed line — passed the active filter at the moment this cache
    /// was last synced (SQ-1179). A repair needs this to decide whether that
    /// line contributed an entry to `starts`/`rows` that must be popped before
    /// re-wrapping the tail: by the time a repair runs, the transcript has
    /// already been mutated, so the CURRENT kind at that raw index describes
    /// whatever moved there, not what the cache actually wrapped.
    pub tail_visible: bool,
    /// The screen-clear anchor mapped into FILTERED line coordinates. Recomputed
    /// on every append or repair (cheaply, over only the newly-wrapped suffix —
    /// see the render path), not merely carried: the anchor itself CAN move
    /// without forcing a rebuild (SQ-1179), as long as it moves to a position at
    /// or after this cache's synced length, in which case every already-cached
    /// filtered line unconditionally precedes it.
    pub clear_anchor_filtered: Option<usize>,
    /// Wrapped-row count before the filtered screen-clear anchor; drives
    /// top-anchoring. Recomputed from `starts` on every append.
    pub anchor_row: Option<usize>,
    /// Arc-ptr set of every inline image present in the filtered transcript, for
    /// bounding the inline-image protocol cache.
    pub live_bands: std::collections::HashSet<usize>,
}

// `WrappedRow` carries images and is not `Debug`; summarise instead of recursing.
impl std::fmt::Debug for CellWrapCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CellWrapCache")
            .field("rows", &self.rows.len())
            .field("stable_rows", &self.stable_rows)
            .field("anchor_row", &self.anchor_row)
            .field("live_bands", &self.live_bands.len())
            .finish()
    }
}

// ── The raster path's product ─────────────────────────────────────────────────

/// A window-0 inline picture placed in ABSOLUTE wrapped-row coordinates, while
/// the raster wrap is running. `RasterFloat` is the same picture shifted into the
/// visible slice; this is the one the wrap itself reserves columns against.
#[derive(Clone)]
pub(crate) struct RasterAbsFloat {
    pub row: usize,
    pub rows: u16,
    /// Columns removed from the text width on the covered rows.
    pub reserve: u16,
    /// Column where covered rows' text begins.
    pub text_col: u16,
    /// Column where the picture blits.
    pub img_col: u16,
    pub img: std::sync::Arc<image::RgbaImage>,
}

/// The cached wrapped-transcript product for the RASTER path, for one
/// [`WrapKey`]. The twin of [`CellWrapCache`]: different product, same rule.
pub(crate) struct RasterWrapCache {
    pub key: WrapKey,
    /// Fully wrapped rows for the whole transcript (oldest-first). The raster
    /// path applies no transcript filter — window 0 is the game's own screen.
    pub rows: Vec<String>,
    /// Per-char §8.7.1 emphasis, parallel to `rows` and SELF-PADDING with empty
    /// (= all-roman) rows, so an unemphasised transcript allocates nothing. May
    /// be shorter than `rows`; read it with `get`.
    pub styles: Vec<Vec<u8>>,
    /// Every float placed, in absolute wrapped-row coordinates. Kept whole rather
    /// than pruned to the visible slice: `reserve_at` consults it while wrapping,
    /// and the visible subset is derived per frame.
    pub floats: Vec<RasterAbsFloat>,
    /// Wrapped row each source line starts at — the raster twin of
    /// [`CellWrapCache::starts`].
    pub starts: Vec<usize>,
}

impl std::fmt::Debug for RasterWrapCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterWrapCache")
            .field("rows", &self.rows.len())
            .field("floats", &self.floats.len())
            .finish()
    }
}

/// The raster story wrap: extend `cache` with source lines from `from` onwards.
///
/// Pulled out of `build_main_text` whole (SQ-1034) so the wrap and the windowing
/// are separable — the windowing is per frame and cheap, the wrap is per new line
/// and was per frame. The body is the wrap it always was.
///
/// **By PIXEL, not by column** (SQ-1009). Every full prose line in
/// `machine-screenshots/amiga-arthur-text.png` ends within one word's width of
/// the same margin — 596 to 634 device px — while carrying different character
/// counts, 71 on the first and fewer on the fourth. No column count reproduces
/// that, and the wrap width is what makes proportional text FILL a line rather
/// than stopping two thirds of the way across.
///
/// The prose wrap is the HOST's: window 0 is wrap+scroll, so it falls past zvm's
/// grid path to the stream and the game never learns where the breaks fell.
///
/// Identical to the character arithmetic it replaced for every fixed-pitch face,
/// which is every configuration but Arthur's Amiga floppy: `run_px` is
/// `chars * cell.w` there and the comparison scales by `font_w` on both sides.
pub(crate) fn raster_wrap_extend(cache: &mut RasterWrapCache, state: &AppState, cols: u16, from: usize) {
    // Non-square 8x16 v6 cell (SQ-0479). Picture pixels arriving here are already
    // in unit space (session scales v6 art x2 before storing), so a float spans
    // height/font_h text rows and indents width/font_w columns.
    let font_w = u32::from(state.v6_text.cell().w());
    let font_h = u32::from(state.v6_text.cell().h());
    // A prose column narrower than this (cells) isn't worth floating a picture
    // beside — fall back to a full-width band.
    const MIN_TEXT_COLS: u16 = 8;
    // The §8.7.1 style bits the raster font can synthesize a face for (SQ-0540):
    // bold and italic. Reverse and fixed-pitch are meaningless in the prose raster
    // (no block to swap, one fixed-pitch face) and are dropped here.
    const EMPHASIS: u8 = 2 | 4;

    // Columns reserved (subtracted from wrap width) by any float covering `row`.
    fn reserve_at(floats: &[RasterAbsFloat], row: usize) -> u16 {
        floats
            .iter()
            .filter(|f| f.row <= row && row < f.row + f.rows as usize)
            .map(|f| f.reserve)
            .max()
            .unwrap_or(0)
    }
    fn set_row_styles(styles: &mut Vec<Vec<u8>>, row: usize, bits: Vec<u8>) {
        if bits.iter().all(|&b| b == 0) {
            return;
        }
        if styles.len() <= row {
            styles.resize(row + 1, Vec::new());
        }
        styles[row] = bits;
    }

    let wrapped = &mut cache.rows;
    let wrapped_styles = &mut cache.styles;
    let floats = &mut cache.floats;
    let line_starts = &mut cache.starts;

    for (i, line) in state.transcript.iter().enumerate().skip(from) {
        line_starts.push(wrapped.len());
        if let Some(Some(img)) = state.transcript_images.get(i) {
            // SQ-0461's `ContentSplash` skip stood here: the raster path draws the
            // graphics window canvas itself, so a band anchored for the frameless
            // mode had to be stepped over or the art drew twice. SQ-0895 removed
            // the mode and with it the only emitter, so there is nothing left to
            // skip — every entry reaching this point is window-0 story content.
            let px = &img.pixels;
            // Rows the picture spans: ceil(h/FONT), so a picture whose height
            // isn't a cell multiple never has a full-width line drawn across its
            // bottom pixels. (Infocom's own countdown used floor and let the
            // overlap happen; with our whole-cell glyphs the ceil reads far
            // cleaner.)
            let img_rows = (px.height().div_ceil(font_h) as u16).max(1);
            let img_cols = (px.width().div_ceil(font_w) as u16).max(1);
            let band = |floats: &mut Vec<RasterAbsFloat>, wrapped: &mut Vec<String>| {
                // A full-width band: reserve blank text rows so the wrap below
                // can't place prose beside or over the picture.
                floats.push(RasterAbsFloat {
                    row: wrapped.len(),
                    rows: img_rows,
                    reserve: 0,
                    text_col: 0,
                    img_col: 0,
                    img: std::sync::Arc::clone(px),
                });
                for _ in 0..img_rows {
                    wrapped.push(String::new());
                }
            };
            match img.align {
                crate::inline_image::ImageAlign::MarginLeft => {
                    // A drop-cap floats at the LEFT: it occupies no text row of
                    // its own — the wrap below narrows the rows beside it, and the
                    // text is pushed right past the picture.
                    let indent_px = img.margin_px.unwrap_or(px.width() + font_w);
                    let reserve = indent_px.div_ceil(font_w) as u16;
                    floats.push(RasterAbsFloat {
                        row: wrapped.len(),
                        rows: img_rows,
                        reserve,
                        text_col: reserve,
                        img_col: 0,
                        img: std::sync::Arc::clone(px),
                    });
                }
                crate::inline_image::ImageAlign::MarginRight => {
                    // A right-margin picture (Shogun's opening, ZMSD §15) floats at
                    // the RIGHT edge: text stays flush left and wraps in the
                    // narrowed column, then reclaims full width once the picture
                    // ends. Reserve the picture's own cell width plus a gutter; if
                    // that leaves no prose column, fall back to a full-width band.
                    let reserve = (img_cols + 1).min(cols);
                    if cols.saturating_sub(reserve) >= MIN_TEXT_COLS {
                        floats.push(RasterAbsFloat {
                            row: wrapped.len(),
                            rows: img_rows,
                            reserve,
                            text_col: 0,
                            img_col: cols.saturating_sub(img_cols),
                            img: std::sync::Arc::clone(px),
                        });
                    } else {
                        band(floats, wrapped);
                    }
                }
                _ => band(floats, wrapped),
            }
            continue;
        }
        if line.is_empty() {
            wrapped.push(String::new());
            continue;
        }
        // Per-char emphasis for this logical line (SQ-0540), materialised only
        // when the line actually carries some — `transcript_runs` is parallel to
        // `transcript`, with char offsets into the UNWRAPPED line.
        let line_bits: Option<Vec<u8>> = state.transcript_runs.get(i).and_then(|runs| {
            runs.iter().any(|r| r.bits & EMPHASIS != 0).then(|| {
                let n = line.chars().count();
                let mut v = vec![0u8; n];
                for r in runs {
                    let end = r.end.min(n);
                    for b in v.iter_mut().take(end).skip(r.start.min(end)) {
                        *b = r.bits & EMPHASIS;
                    }
                }
                v
            })
        });
        // Slice `line_bits` for a wrapped row of `n` chars starting at source char
        // offset `from`. Wrapping only ever drops the single space at a break, so
        // each row is a contiguous run of the source line.
        let row_bits = |from: usize, n: usize| -> Vec<u8> {
            match &line_bits {
                Some(bits) => (0..n).map(|j| bits.get(from + j).copied().unwrap_or(0)).collect(),
                None => Vec::new(),
            }
        };
        // Word-wrap with per-row width: rows beside an active float are narrower.
        let tf = &state.v6_text;
        // **A row is measured in the style it will be DRAWN in** (SQ-1050).
        //
        // `run_px` is `run_px_styled(s, 0)` — the ROMAN pen — and `draw_story_text`
        // steps each glyph by `advance_styled(ch, row_styles[col])`. So the wrap
        // was asking a different question than the draw answered, and a bold row
        // ran past the edge by exactly the smear it had not been measured with.
        //
        // Amiga Arthur is the report (the user's `bold-overflow` state, r54/890606
        // off `Arthur - The Quest for Excalibur.adf`, one `wait` after the restore):
        // `[You have earned five experience points and two quest points.]` measures
        // **564 px roman**, fits the 584 px box, and is drawn at **688 px** — 104 px
        // past the right edge, thirteen characters' worth. `zvm`'s own pen, which
        // has measured with the style since SQ-1009, breaks the same line at 50
        // characters / 574 px, which is the answer this now agrees with.
        //
        // Italic costs nothing here and is measured all the same: `V6Metric::advance`
        // widens for `STYLE_BOLD` only, so an italic row answers the roman width on
        // both sides and stays byte-identical. `line_bits` carries no other bit —
        // reverse and fixed-pitch are masked off above — so this cannot reach for a
        // fixed alternate the prose is not drawn with.
        let span_px = |from: usize, s: &str| -> u32 {
            match &line_bits {
                Some(bits) => s
                    .chars()
                    .enumerate()
                    .map(|(j, c)| tf.advance_styled(c, bits.get(from + j).copied().unwrap_or(0)))
                    .sum(),
                None => tf.run_px(s),
            }
        };
        // The separating blank is the source line's own character, so it is measured
        // at ITS bits too — a bold space is a wider space.
        let gap_px = |at: usize| -> u32 {
            let bit = line_bits.as_ref().and_then(|b| b.get(at).copied()).unwrap_or(0);
            tf.advance_styled(' ', bit)
        };
        let mut cur = String::new();
        let mut cur_start = 0usize; // source char offset of `cur`'s first char
        let mut src = 0usize; // source char offset of `word`
        for word in line.split(' ') {
            let width = (cols.saturating_sub(reserve_at(floats, wrapped.len())).max(1) as u32) * font_w;
            let gap = src.checked_sub(1).map_or_else(|| tf.advance(' '), gap_px);
            if !cur.is_empty() && span_px(cur_start, &cur) + gap + span_px(src, word) > width {
                let n = cur.chars().count();
                wrapped.push(std::mem::take(&mut cur));
                let row = wrapped.len() - 1;
                set_row_styles(wrapped_styles, row, row_bits(cur_start, n));
            }
            if cur.is_empty() {
                cur_start = src;
            } else {
                cur.push(' ');
            }
            cur.push_str(word);
            src += word.chars().count() + 1; // +1 for the separating space
        }
        let n = cur.chars().count();
        wrapped.push(cur);
        let row = wrapped.len() - 1;
        set_row_styles(wrapped_styles, row, row_bits(cur_start, n));
    }
}

/// Bring `state.raster_wrap` up to date for `cols` and return nothing — the
/// caller reads the cache. The whole append-or-rebuild decision is
/// [`WrapKey::plan`]'s; this is the raster half of obeying it.
pub(crate) fn raster_wrap_refresh(state: &AppState, cols: u16) {
    let mut slot = state.raster_wrap.borrow_mut();
    let plan = match slot.as_ref() {
        Some(c) => c.key.plan(state, cols),
        None => WrapPlan::Rebuild,
    };
    match plan {
        WrapPlan::Reuse => {}
        WrapPlan::Append { from } => {
            let cache = slot.as_mut().expect("a plan against a cached key");
            raster_wrap_extend(cache, state, cols, from);
            cache.key = WrapKey::of(state, cols);
        }
        // SQ-1179's tail repair is a CellWrapCache mechanism (float `carry`
        // entering a discarded line, `tail_visible` bookkeeping) that raster
        // has no equivalent for — the same product-correctness answer as a
        // full rebuild, only cheaper, and raster's rebuild is already cheap
        // (its wrap width is the constant native prose box, so it takes this
        // branch essentially never — see this module's own doc comment).
        WrapPlan::Rebuild | WrapPlan::Repair { .. } => {
            let mut cache = RasterWrapCache {
                key: WrapKey::of(state, cols),
                rows: Vec::new(),
                styles: Vec::new(),
                floats: Vec::new(),
                starts: Vec::with_capacity(state.transcript.len() + 1),
            };
            raster_wrap_extend(&mut cache, state, cols, 0);
            *slot = Some(cache);
        }
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::render::screen::build_main_text;
    use crate::state::TranscriptKind;

    /// ONE picture, shared by both routes through the script — see the cell
    /// path's twin in `render::transcript`'s tests for why identity matters.
    fn script_image() -> crate::inline_image::InlineImage {
        static IMG: std::sync::OnceLock<crate::inline_image::InlineImage> = std::sync::OnceLock::new();
        IMG.get_or_init(|| crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(24, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(32),
        })
        .clone()
    }

    fn script_state(honor: bool) -> AppState {
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = honor;
        state
    }

    /// Emphasised runs covering the whole line, so the per-char emphasis vector
    /// (which is parallel to the wrapped rows and must append in step) is real.
    fn bold(text: &str) -> Vec<crate::state::StyleRun> {
        vec![crate::state::StyleRun {
            start: 0,
            end: text.chars().count(),
            bits: 2,
            ..Default::default()
        }]
    }

    /// The same script the cell path's equivalence tests drive, in raster's terms:
    /// prose that wraps, hard newlines, an emphasised run, and a left-margin float
    /// with prose arriving after it.
    fn drive_script(state: &mut AppState, cols: u16, rows: u16, render_after_each: bool) {
        let step = |state: &AppState| {
            if render_after_each {
                std::hint::black_box(build_main_text(state, cols, rows));
            }
        };
        state.push_transcript_kind("You are standing in an open field west of a white house.", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("one\ntwo\nthree", TranscriptKind::Story);
        step(state);
        let em = "an emphasised sentence long enough to wrap more than once across the prose column";
        state.push_transcript_kind(em, TranscriptKind::Story);
        *state.transcript_runs.last_mut().unwrap() = bold(em);
        state.push_transcript_kind("", TranscriptKind::Story); // force the runs edit to settle
        step(state);
        state.push_transcript_image(script_image());
        step(state);
        state.push_transcript_kind("beside one", TranscriptKind::Story);
        step(state);
        state.push_transcript_kind(&"word ".repeat(30), TranscriptKind::Story);
        step(state);
        state.push_transcript_kind("tail", TranscriptKind::Story);
        step(state);
    }

    /// The whole raster product, projected to something comparable.
    fn raster_product(state: &AppState, cols: u16, rows: u16) -> String {
        let (main, metrics) = build_main_text(state, cols, rows);
        let floats: Vec<String> = main
            .floats
            .iter()
            .map(|f| {
                format!(
                    "{:p}/{}x{}@{}+{}+{}",
                    std::sync::Arc::as_ptr(&f.img),
                    f.reserve_cols,
                    f.rows,
                    f.row,
                    f.text_col,
                    f.img_col
                )
            })
            .collect();
        format!(
            "{:?}\n{:?}\n{floats:?}\n{:?}\n{}\n{}\n{:?}",
            main.lines, main.styles, metrics, main.input, main.cursor_col, main.awaiting
        )
    }

    #[test]
    fn raster_appending_lands_on_exactly_what_a_rebuild_would_have_produced() {
        // The raster wrap resolves no colour — it carries §8.7.1 emphasis bits and
        // nothing else — but it is a render path, so both game-colour modes are
        // pinned rather than one (CLAUDE.md), and the palette is stated rather
        // than inherited from whatever a sibling case last booted.
        let _g = crate::v6_palette(zvm::screen::Palette::Standard);
        for honor in [true, false] {
            let (cols, rows) = (48u16, 14u16);

            let mut incremental = script_state(honor);
            drive_script(&mut incremental, cols, rows, true);

            let mut rebuilt = script_state(honor);
            drive_script(&mut rebuilt, cols, rows, false);

            let want = raster_product(&rebuilt, cols, rows);
            // Non-vacuity by shape: the float and the emphasis must both be there,
            // or the comparison is of a wrap that exercised neither.
            let (probe, _) = build_main_text(&rebuilt, cols, rows);
            assert_eq!(probe.floats.len(), 1, "the float must reach the visible slice: {want}");
            assert!(
                probe.styles.iter().any(|row| row.iter().any(|&b| b & 2 != 0)),
                "emphasis bits must reach the wrapped rows: {want}"
            );
            assert_eq!(
                raster_product(&incremental, cols, rows),
                want,
                "seven appends must produce the same raster rows as one rebuild (honor={honor})"
            );
        }
    }

    #[test]
    fn raster_appending_across_a_width_change_lands_on_exactly_what_a_rebuild_would_have_produced() {
        // Raster's `cols` come from the NATIVE v6 screen rect and do not move with
        // the pane, so this is the branch production essentially never takes — and
        // therefore the one that would rot unseen. A restore into a different
        // terminal size is the field case that reaches it, through
        // `reconcile_restored_screen_size` changing the native rect.
        let _g = crate::v6_palette(zvm::screen::Palette::Standard);
        for honor in [true, false] {
            let (narrow, wide, rows) = (32u16, 60u16, 14u16);

            let mut incremental = script_state(honor);
            drive_script(&mut incremental, narrow, rows, true);
            std::hint::black_box(build_main_text(&incremental, wide, rows));
            incremental.push_transcript_kind(&"after the change ".repeat(4), TranscriptKind::Story);
            std::hint::black_box(build_main_text(&incremental, wide, rows));

            let mut rebuilt = script_state(honor);
            drive_script(&mut rebuilt, narrow, rows, false);
            rebuilt.push_transcript_kind(&"after the change ".repeat(4), TranscriptKind::Story);

            assert_eq!(
                raster_product(&incremental, wide, rows),
                raster_product(&rebuilt, wide, rows),
                "a width change mid-stream must leave the same rows a fresh wrap gives (honor={honor})"
            );
        }
    }

    /// What every restore path does to the transcript, in the order it does it.
    ///
    /// All four sites agree on this shape — `engine_helpers::apply_archive_state`,
    /// `main.rs`'s named-slot restore and history jump, `turn.rs`'s launch resume
    /// and `startup.rs`'s auto-resume: replace the five parallel vecs, then
    /// `reset_transcript_sidecars`, then re-attach the images the reset zeroed.
    /// The reset is the only one of those that touches the wrap cache's content
    /// signal, which is why it is the one that has to say `Rewrote`.
    fn restore_transcript(state: &mut AppState, lines: &[&str]) {
        state.transcript = lines.iter().map(|s| s.to_string()).collect();
        state.clear_anchor = None;
        state.transcript_kinds = vec![TranscriptKind::Story; lines.len()];
        state.transcript_runs = vec![Vec::new(); lines.len()];
        state.transcript_para = vec![crate::state::ParaFmt::default(); lines.len()];
        state.reset_transcript_sidecars();
    }

    #[test]
    fn a_restore_into_a_different_size_and_backend_rebuilds_rather_than_appending() {
        // A restore into a different terminal size and a different graphics
        // backend is common in the field and invisible to a same-session
        // round-trip (CLAUDE.md), and it is the one case where the transcript both
        // SHRINKS and changes underneath a warm cache.
        //
        // Asserted one move AFTER the restore, never on the frame it lands: a
        // restore that quietly appended onto the pre-restore scrollback shows the
        // archive's own rows correctly until something is printed into them.
        let _g = crate::v6_palette(zvm::screen::Palette::Standard);
        for honor in [true, false] {
            let rows = 14u16;
            let (before_cols, after_cols) = (48u16, 33u16);

            let mut live = script_state(honor);
            live.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            drive_script(&mut live, before_cols, rows, true);

            restore_transcript(&mut live, ARCHIVED);
            // The restore lands on a different pane and a different backend.
            live.game_picker = Some(crate::render::graphics::kitty_picker(8, 16));
            std::hint::black_box(build_main_text(&live, after_cols, rows));
            // PERTURB: the game's next turn prints into the restored screen.
            live.push_transcript_kind("You open the mailbox, revealing a small leaflet.", TranscriptKind::Story);
            std::hint::black_box(build_main_text(&live, after_cols, rows));

            let mut fresh = script_state(honor);
            fresh.game_picker = Some(crate::render::graphics::kitty_picker(8, 16));
            restore_transcript(&mut fresh, ARCHIVED);
            fresh.push_transcript_kind("You open the mailbox, revealing a small leaflet.", TranscriptKind::Story);

            let want = raster_product(&fresh, after_cols, rows);
            assert!(
                want.contains("leaflet"),
                "non-vacuity: the move after the restore must be on screen: {want}"
            );
            assert!(
                !want.contains("beside one"),
                "non-vacuity: the pre-restore scrollback must be GONE: {want}"
            );
            assert_eq!(
                raster_product(&live, after_cols, rows),
                want,
                "a restore then a move must leave the archive's rows, not the old ones (honor={honor})"
            );
        }
    }

    /// A restored transcript: shorter than the one it replaces, and sharing none
    /// of its lines — so appending onto the old rows instead of rebuilding is
    /// visible rather than merely wrong.
    const ARCHIVED: &[&str] = &[
        "West of House",
        "You are standing in an open field west of a white house, with a boarded front door.",
        "There is a small mailbox here.",
        "",
        ">",
    ];

    #[test]
    fn an_in_place_edit_of_the_last_line_is_caught_even_when_it_is_misclassified() {
        // The guard, tested as a guard (CLAUDE.md: a guard beats a convention).
        //
        // `WrapContent::edits` is stated by hand at each mutator and the next
        // mutator is written by someone with no reason to know that. So the tail
        // fingerprint is asked the question a misclassification would answer
        // wrongly: the transcript is edited in place with `transcript_edits` held
        // deliberately still, exactly as a mutator that picked `Appended` would
        // leave it, and the plan must still say Rebuild.
        let mut state = script_state(true);
        state.push_transcript_kind("hello", TranscriptKind::Story);
        let key = WrapKey::of(&state, 40);
        assert_eq!(key.plan(&state, 40), WrapPlan::Reuse, "nothing moved");

        let edits = state.transcript_edits;
        state.transcript.last_mut().unwrap().push_str(" world");
        state.transcript_edits = edits; // the misclassification, made by hand
        assert_eq!(
            key.plan(&state, 40),
            WrapPlan::Rebuild,
            "the tail fingerprint must catch an edit `edits` was never told about"
        );
    }

    #[test]
    fn an_interior_line_that_moves_is_caught_by_the_edit_kind_the_mutator_stated() {
        // The case that separates the mechanism from its guard. The tail
        // fingerprint watches the LAST consumed line, which is where every
        // in-place mutator we have happens to work — so it catches almost
        // everything. What it cannot see is a mutation in the MIDDLE of the
        // scrollback that leaves the length and the last line alone, and
        // `turn.rs` performs exactly one: `fill_line_default_colours` re-grounds
        // the folded self-echo at `before_push - 1`, with the turn's own output
        // already pushed after it.
        //
        // Nothing about the transcript's shape reports that. Only
        // `TranscriptEdit::Rewrote`, stated at the mutator, does.
        let mut state = script_state(true);
        state.push_transcript_kind("first\nsecond\nthird", TranscriptKind::Story);
        let key = WrapKey::of(&state, 40);
        assert_eq!(key.plan(&state, 40), WrapPlan::Reuse, "nothing moved yet");

        let white = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(9));
        let blue = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(6));
        state.fill_line_default_colours(0, white, blue);
        assert_eq!(state.transcript.len(), 3, "the length must not move, or this proves nothing");
        assert_eq!(
            state.transcript.last().map(String::as_str),
            Some("third"),
            "the tail must not move either, or the fingerprint would catch it instead"
        );
        assert_eq!(
            key.plan(&state, 40),
            WrapPlan::Rebuild,
            "a colour fill on an interior line must rebuild the wrap it restyled"
        );
    }

    #[test]
    fn the_live_input_line_is_not_part_of_the_key() {
        // The defect this quest exists to remove, stated as a property: the input
        // line is not in the wrapped product at all — `build_main_text` takes it
        // as a separate argument — so typing must not invalidate a wrap.
        let mut state = script_state(true);
        state.push_transcript_kind("hello", TranscriptKind::Story);
        let key = WrapKey::of(&state, 40);
        for ch in "open mailbox".chars() {
            state.input.value.push(ch);
            assert_eq!(key.plan(&state, 40), WrapPlan::Reuse, "a keystroke must not move the wrap");
        }
    }
}
