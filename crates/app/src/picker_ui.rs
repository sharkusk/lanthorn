//! Story-picker UI subsystem: the pre-game story browser and its metadata
//! info panel. Extracted verbatim from `main.rs` (SQ-0306) as the UI companion
//! to the `app::picker` logic module. Pure move — no behavior change.

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{read, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use app::anim::PanelSlide;
use app::render::draw_str_clipped;
use app::render::panel::{draw_panel, PanelSpec, PanelStrip};
use app::render::paneframe::{InsetSegment, PaneGlyphs};

use crate::{abbreviate_home, exit_if_terminated, restore_terminal};

/// Minimum column widths for the story list and info panel, respectively.
/// The panel refuses to open when the terminal is narrower than their sum.
const LIST_MIN_W: u16 = 24;
const PANEL_MIN_W: u16 = 28;

/// Which story-picker view is active. `List` is the metadata table (default);
/// `Gallery` is the cover-thumbnail grid (SQ-0374). Toggled with `g`; the info
/// panel toggles independently (`i`/`Tab`) in both views.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerView {
    List,
    Gallery,
}

/// A previewable bundled resource the info panel links to (SQ-0347): an image
/// (`Pict`) or a sound (`Snd `). Carries where to re-read the bytes from (the
/// story's own blorb, or its sidecar) since the panel's `ChunkInfo` list holds
/// only display strings, not the resource data.
#[derive(Clone)]
struct ResourceRef {
    blorb_path: std::path::PathBuf,
    kind: PreviewKind,
    number: u32,
    label: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviewKind {
    Image,
    Sound,
}

/// A bundled resource being shown in the picker's preview modal (SQ-0347): a
/// decoded image (rebuilt to a fitted protocol as the modal draws) and/or a
/// one-line status (sound playback result, or why an image can't be shown).
struct ResourcePreview {
    /// Dialog title, e.g. `"Image #7"`.
    title: String,
    /// The decoded image, when this is a renderable Pict.
    image: Option<image::DynamicImage>,
    /// Cached protocol, keyed by the content rect `(w, h)` and zoom it was
    /// built for, so a resize or a zoom step invalidates it. The trailing
    /// `Option<u32>` is the kitty image id it was last placed under (SQ-1190),
    /// set by `draw_resource_preview` right after `place_protocol`. Freeing it
    /// (on rebuild, or when the modal closes) rides on `cover`'s own delete
    /// queue — the preview has no `GraphicsRender` to share either, and reusing
    /// `cover`'s (already flushed every frame) beats a second private one that
    /// would need flushing too, and would lose whatever it held whenever the
    /// whole `ResourcePreview` is dropped on close.
    proto: Option<(u16, u16, PreviewZoom, ratatui_image::protocol::Protocol, Option<u32>)>,
    /// A status line shown instead of (or below) an image.
    status: Option<String>,
    /// Current zoom (SQ-0486): `Fit` on open; `+`/`-`/wheel step it.
    zoom: PreviewZoom,
}

/// Image-preview zoom (SQ-0486). `Fit` scales the image (up or down, at
/// whatever ratio) to fill the content rect — today's default on open.
/// `Factor(n)` renders at an exact integer multiple of the image's native
/// pixel size, nearest-neighbour scaled so pixel art stays crisp; when the
/// scaled result overflows the content rect it is centre-cropped rather than
/// shrunk back down (zooming small art up past "fit" is the whole point).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviewZoom {
    Fit,
    Factor(u32),
}

/// Largest integer zoom factor the modal allows: generous enough to blow up
/// postage-stamp 320×200-era art, capped so it can't allocate an absurd bitmap.
const MAX_ZOOM_FACTOR: u32 = 16;

impl PreviewZoom {
    /// One zoom-in step (`+`/`=`, wheel up): `Fit` jumps to native size (1×),
    /// then each step adds one factor up to `MAX_ZOOM_FACTOR`.
    fn step_in(self) -> Self {
        match self {
            PreviewZoom::Fit => PreviewZoom::Factor(1),
            PreviewZoom::Factor(n) => PreviewZoom::Factor((n + 1).min(MAX_ZOOM_FACTOR)),
        }
    }

    /// One zoom-out step (`-`, wheel down): 1× drops back to `Fit`; `Fit`
    /// stays `Fit` (there's nothing below the default).
    fn step_out(self) -> Self {
        match self {
            PreviewZoom::Fit => PreviewZoom::Fit,
            PreviewZoom::Factor(1) => PreviewZoom::Fit,
            PreviewZoom::Factor(n) => PreviewZoom::Factor(n - 1),
        }
    }

    /// Chrome label, e.g. `"Fit"` / `"3×"`.
    fn label(self) -> String {
        match self {
            PreviewZoom::Fit => "Fit".to_string(),
            PreviewZoom::Factor(n) => format!("{n}\u{d7}"),
        }
    }
}

/// The rect (in the *scaled* image's own pixel space) to pull from a
/// `native × factor`-scaled image so the result fits within `budget` pixels —
/// the centre-crop math backing `PreviewZoom::Factor` overflow (SQ-0486, req
/// 2). `w`/`h` never exceed `budget` or `scaled`; a `scaled` already smaller
/// than `budget` crops nothing (`x == y == 0`, full image).
fn center_crop_rect(scaled: (u32, u32), budget: (u32, u32)) -> (u32, u32, u32, u32) {
    let w = scaled.0.min(budget.0.max(1));
    let h = scaled.1.min(budget.1.max(1));
    let x = (scaled.0.saturating_sub(w)) / 2;
    let y = (scaled.1.saturating_sub(h)) / 2;
    (x, y, w, h)
}

/// Story-list row layout: the selection-marker glyph column, the gap between
/// text columns, and each data column's target width. Rating drops first as
/// the row narrows, then year, then author, leaving title + badges at the
/// narrowest — see `compute_columns`.
const ROW_MARKER_W: u16 = 2;
const COL_GAP: u16 = 2;
const AUTHOR_COL_W: u16 = 20;
const AUTHOR_MAX_W: u16 = 40;
const YEAR_COL_W: u16 = 6;
/// IFDB rating column: the average to one decimal, then the number of votes it
/// is over — `4.6 (118)`. Never stars. The vote count matters because a lone
/// 5.0 and a 5.0 over 300 ratings are not the same claim.
///
/// Sized for `4.6 (1234)` (10) — a 4-digit count is comfortably beyond IFDB's
/// most-rated games, and the value now outgrows the header, so `RATING ▲` (8)
/// fits where the old 6-wide column could only take `RATE ▲`.
const RATING_COL_W: u16 = 10;
/// Interpreter/format column ("Z5", "Z5 (blorb)", "G3.1.2"): fixed width, sits
/// just left of the badge cluster. `Z8 (blorb)` (10) is the widest (SQ-0369).
const INTERP_COL_W: u16 = 13;
const TITLE_MIN_W: u16 = 8;
/// Title keeps this much before the author column is allowed to grow past its
/// base width — title has priority for the shared space, so a long author name
/// never squeezes the title down to `TITLE_MIN_W`.
const TITLE_PREFERRED_W: u16 = 24;

/// Resolved column widths for one draw, given `text_w` — the row width left
/// for marker+title+author+year+rating once the badge cluster's fixed columns
/// (and its lead-in gap) are excluded by the caller. Title always absorbs
/// whatever space the shown columns don't use, so there is never a gap
/// before the badges.
struct ListColumns {
    title_w: u16,
    author_w: u16,
    year_w: u16,
    rating_w: u16,
}

/// `want_author_w` is the widest author display width to show in full; the
/// author column grows from `AUTHOR_COL_W` toward it, but never so far that
/// title would drop below `TITLE_MIN_W`, and never past `AUTHOR_MAX_W` so one
/// very long name can't swallow the row. Title still absorbs any leftover, so
/// there is no gap before the badges.
fn compute_columns(text_w: u16, want_author_w: u16) -> ListColumns {
    let avail = text_w.saturating_sub(ROW_MARKER_W);
    let need_year = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W + COL_GAP + YEAR_COL_W;
    let need_rating = need_year + COL_GAP + RATING_COL_W;
    let need_author = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W;
    let grow = |cols_space: u16| -> u16 {
        // Space shared by title+author (year already excluded). Author gets at
        // least AUTHOR_COL_W and grows toward want_author_w, but only into space
        // left after title keeps TITLE_PREFERRED_W — title has priority, so a
        // long name can't shrink the title below a comfortable width. Capped at
        // AUTHOR_MAX_W so one very long name can't swallow the row either.
        let ceiling = AUTHOR_MAX_W.min(cols_space.saturating_sub(TITLE_PREFERRED_W));
        want_author_w.clamp(AUTHOR_COL_W, ceiling.max(AUTHOR_COL_W))
    };
    if avail >= need_rating {
        let cols_space = avail - COL_GAP - COL_GAP - YEAR_COL_W - COL_GAP - RATING_COL_W;
        let author_w = grow(cols_space);
        ListColumns {
            title_w: cols_space - author_w,
            author_w,
            year_w: YEAR_COL_W,
            rating_w: RATING_COL_W,
        }
    } else if avail >= need_year {
        let cols_space = avail - COL_GAP - COL_GAP - YEAR_COL_W;
        let author_w = grow(cols_space);
        ListColumns { title_w: cols_space - author_w, author_w, year_w: YEAR_COL_W, rating_w: 0 }
    } else if avail >= need_author {
        let cols_space = avail - COL_GAP;
        let author_w = grow(cols_space);
        ListColumns { title_w: cols_space - author_w, author_w, year_w: 0, rating_w: 0 }
    } else {
        ListColumns { title_w: avail, author_w: 0, year_w: 0, rating_w: 0 }
    }
}

/// Short interpreter/format label for the story-list TYPE column, type letter
/// plus the detected VM version: `Z<v>` for Z-code ("Z5", "Z3") and `G<v>` for
/// Glulx ("G3.1.2", from the Glulx header version). A blorb-wrapped Z-machine
/// story gets a " (blorb)" suffix ("Z5 (blorb)"), which subsumes the old B
/// badge; Glulx is omitted since Glulx games are effectively always blorbed
/// (SQ-0369). A Scott game shows "Scott", with " (blorb)" for the graphic
/// `.blb` versions (`.dat` is not blorbed, so the suffix distinguishes them).
/// Bare "Z"/"Glulx" when the version is unknown.
///
/// A story mounted out of a release floppy takes its container's acronym in the
/// same slot — `Z6 (ADF)` off an Amiga disk, `Z6 (HFS)` off a Macintosh one — so
/// a disk image is not mistaken for a bare story file, and one machine's media
/// is not mistaken for another's (SQ-0737, SQ-0837). It is the disk that says
/// so, not the filename: `meta.disk_image` is the mount's own answer. A disk
/// image is never also a blorb, so the two suffixes cannot collide. (The
/// container names keep their own casing: "blorb" is a format name, "ADF" and
/// "HFS" acronyms.)
///
/// The parenthetical itself is [`app::picker::type_container`] — one rule,
/// shared with the TYPE column's sort, which orders rows by the container it
/// names (SQ-1057). Only the base letters are decided here.
fn interp_label(meta: &app::picker::StoryMeta, blorb: bool) -> String {
    let base = match meta.engine {
        app::picker::Engine::ZCode => match meta.version.as_deref() {
            Some(v) if !v.is_empty() => format!("Z{v}"),
            _ => "Z".to_string(),
        },
        app::picker::Engine::Glulx => match meta.version.as_deref() {
            Some(v) if !v.is_empty() => format!("G{v}"),
            _ => "Glulx".to_string(),
        },
        app::picker::Engine::Scott => "Scott".to_string(),
    };
    // Which container this row shows — and whether it shows one at all — is
    // `app::picker::type_container`, the same call the TYPE *sort* makes, so
    // the column and its ordering cannot disagree (SQ-1057).
    match app::picker::type_container(meta, blorb) {
        Some(container) => format!("{base} ({container})"),
        None => base,
    }
}

/// Truncate `s` to at most `max_w` display columns (unicode display width,
/// not char count — a CJK title is 2 cells per char and `chars().count()`
/// would misalign every column to its right), appending `…` when it doesn't
/// fit.
fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_w {
        return s.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let target = max_w - 1; // room for the 1-wide ellipsis
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > target {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Word-wrap `s` to at most `width` display columns per line (unicode-aware,
/// same width rule as `truncate_to_width`), splitting greedily on whitespace.
/// A blank line in `s` (a paragraph break) is preserved as an empty output
/// line. A single word wider than `width` is placed on its own line rather
/// than broken mid-word — same as any other overlong field, it is left for
/// the renderer to clip. `width == 0` returns `s` verbatim as one line.
fn wrap_to_width(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in para.split_whitespace() {
            let word_w = UnicodeWidthStr::width(word);
            let sep_w = if cur.is_empty() { 0 } else { 1 };
            if !cur.is_empty() && cur_w + sep_w + word_w > width {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(word);
            cur_w += word_w;
        }
        lines.push(cur);
    }
    lines
}

/// Columns the info panel indents a wrapped continuation row by, and the marker
/// drawn in them (SQ-0861). Fixed at two cells because the indent arithmetic
/// depends on the marker's width — it is themeable through the
/// `story_info_continuation` selector, not swappable for a wider glyph.
const PANEL_CONT_INDENT: usize = 2;
const PANEL_CONT_MARK: &str = "↳ ";

/// One DRAWN row of the info panel, after wrapping (SQ-0861).
///
/// The panel builds a flat list of logical lines and used to draw one row per
/// line, so anything wider than the panel — a compilation's `…(Disk 6 of
/// 7).2mg:LEATHRGODDESSES` file line, a UUID-form IFID, the `Saves · <dir>`
/// header, a save row ending in a filename — was simply clipped at the edge.
/// Wrapping turns one logical line into one or more of these; `src` is the
/// logical line's index, so the link and resource tables (which are keyed by
/// logical index) still resolve without remapping.
struct PanelRow {
    text: String,
    style: ratatui::style::Style,
    /// A wrapped continuation of the row above, drawn indented behind a marker.
    cont: bool,
    src: usize,
}

/// Break `s` into rows of at most `first_w` display CELLS for the first row and
/// `cont_w` for every row after it (SQ-0861).
///
/// Width is measured in terminal columns via `textwidth::row_break`, not in
/// bytes or chars, so a CJK title or a path carrying combining marks wraps where
/// it actually reaches the panel edge and a double-width glyph is never split.
/// Words stay whole where a space allows it; a token wider than the row — which
/// is what a long filename is — is broken at the cell boundary rather than left
/// for the renderer to clip, because clipping is the defect being fixed.
///
/// Always terminates: every iteration consumes at least one char, including the
/// degenerate case of a double-width glyph in a one-column row (nothing "fits",
/// so the glyph is taken anyway and overflows by one cell). A zero width is the
/// one case that cannot make progress at all, and returns `s` unwrapped.
fn wrap_panel_line(s: &str, first_w: usize, cont_w: usize) -> Vec<String> {
    if first_w == 0 || cont_w == 0 {
        return vec![s.to_string()];
    }
    let mut rows: Vec<String> = Vec::new();
    let mut rest = s;
    loop {
        let w = if rows.is_empty() { first_w } else { cont_w };
        let br = app::textwidth::row_break(rest, w);
        let Some(overflow) = br.overflow else {
            rows.push(rest.to_string());
            break;
        };
        // Prefer the last space at or before the break so words stay whole. A
        // space at offset 0 is not a break point — it would emit an empty row
        // and re-present the same remainder forever.
        let (take, mut skip, broke_on_space) = match br.last_space.filter(|b| *b > 0) {
            Some(b) => (b, b + 1, true),
            None if overflow > 0 => (overflow, overflow, false),
            // Nothing fits at all: a glyph wider than the row. Take it whole so
            // the scan advances; the renderer clips its overhanging cell.
            None => {
                let n = rest.chars().next().map_or(0, char::len_utf8);
                (n, n, false)
            }
        };
        if broke_on_space {
            // The panel's own separators include double spaces (`… turn 42 ·
            // 2026-06-30  save.lanthorn`), so a break can land inside a RUN of
            // them: none of that run belongs to either row.
            rows.push(rest[..take].trim_end_matches(' ').to_string());
            skip += rest[skip..].len() - rest[skip..].trim_start_matches(' ').len();
        } else {
            rows.push(rest[..take].to_string());
        }
        rest = &rest[skip..];
        if rest.is_empty() {
            break;
        }
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Column header text plus whether it's the active sort column — the
/// direction arrow is shown only on the active column.
fn header_label(name: &str, key: app::picker::SortKey, sort: app::picker::Sort) -> (String, bool) {
    if sort.key == key {
        let arrow = if sort.desc { "▼" } else { "▲" };
        (format!("{name} {arrow}"), true)
    } else {
        (name.to_string(), false)
    }
}

/// The gap between two footer hints.
const FOOTER_GAP: &str = "  ";

/// The droppable footer segments in KEEP order — the last to go as the terminal
/// narrows comes first, which is also the order they come back as it widens.
///
/// A hint whose command nobody has bound simply is not shown; the footer has no
/// way to claim a key that does not exist, which is the drift SQ-0796 set out to
/// end.
#[cfg(test)]
fn footer_optional(km: &app::keymap::KeyMap, gallery: bool) -> Vec<String> {
    let mut hints: Vec<app::browser::Hint> = app::browser::footer_hints(gallery)
        .into_iter()
        .filter(|h| h.drop_rank.is_some())
        .collect();
    hints.sort_by_key(|h| std::cmp::Reverse(h.drop_rank.unwrap_or(0)));
    hints.iter().filter_map(|h| app::browser::render_hint(km, h)).collect()
}

/// Build the footer for `width` (SQ-1227).
///
/// `Enter: open`, `Space: menu` and `q: quit` are always shown — the first two
/// are how anything else is discovered and the third is the way out. The rest
/// are added in `drop_rank` order (highest first) while they still fit, and
/// DRAWN in the table's fixed left-to-right order however many of them survived,
/// so the line never rearranges itself as the window is dragged. Every key comes
/// from the live keymap, so rebinding one relabels its hint (SQ-0796).
fn build_footer(km: &app::keymap::KeyMap, width: u16, gallery: bool) -> String {
    let hints = app::browser::footer_hints(gallery);
    let rendered: Vec<Option<String>> =
        hints.iter().map(|h| app::browser::render_hint(km, h)).collect();
    let mut shown: Vec<bool> = hints.iter().map(|h| h.drop_rank.is_none()).collect();

    let line = |shown: &[bool]| -> String {
        let segs: Vec<&str> = rendered
            .iter()
            .enumerate()
            .filter(|(i, _)| shown[*i])
            .filter_map(|(_, r)| r.as_deref())
            .collect();
        format!(" {}", segs.join(FOOTER_GAP))
    };

    let mut order: Vec<usize> = (0..hints.len()).filter(|&i| hints[i].drop_rank.is_some()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(hints[i].drop_rank.unwrap_or(0)));
    for i in order {
        if rendered[i].is_none() {
            continue;
        }
        shown[i] = true;
        if UnicodeWidthStr::width(line(&shown).as_str()) as u16 > width {
            // One that does not fit takes everything below it with it: the drop
            // order is an order, not a packing problem.
            shown[i] = false;
            break;
        }
    }
    line(&shown)
}

/// True if the terminal is wide enough to show list + panel.
fn can_open_panel(width: u16) -> bool {
    width >= LIST_MIN_W + PANEL_MIN_W
}

/// Split `area` into (list, panel) given an eased open fraction in `[0,1]`.
/// Panel target width is a third of the area, clamped to
/// `[PANEL_MIN_W, area.width - LIST_MIN_W]`; the eased width is that × fraction.
fn split_picker_area(area: Rect, fraction: f64) -> (Rect, Rect) {
    if fraction <= 0.0 || !can_open_panel(area.width) {
        return (area, Rect::new(area.right(), area.y, 0, area.height));
    }
    let target = (area.width / 3).clamp(PANEL_MIN_W, area.width - LIST_MIN_W);
    let panel_w = ((target as f64) * fraction).round() as u16;
    let panel_w = panel_w.min(area.width - LIST_MIN_W);
    let list_w = area.width - panel_w;
    let list_area = Rect::new(area.x, area.y, list_w, area.height);
    let panel_area = Rect::new(area.x + list_w, area.y, panel_w, area.height);
    (list_area, panel_area)
}

/// Resolve and cache the aux data for `idx` if not already cached.
fn ensure_aux(
    cache: &mut [Option<app::picker::StoryAux>],
    stories: &[app::picker::StoryEntry],
    idx: usize,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) {
    if let Some(slot) = cache.get_mut(idx) {
        if slot.is_none() {
            // A folder has no aux to resolve (and nothing to open).
            if let Some(entry) = stories.get(idx).filter(|e| !e.is_folder()) {
                *slot = Some(app::picker::resolve_aux(entry, data_base, hint_index));
            }
        }
    }
}

/// The rows the picker lists for `dir`: a library shows the folder at `dir`
/// (its sub-folders, then its stories, `..` below the root); a multi-disk set
/// is one release and has no folders to show.
fn rows_for(
    source: &app::picker::StorySource,
    dir: &std::path::Path,
    root: &std::path::Path,
    data_base: &std::path::Path,
) -> Vec<app::picker::StoryEntry> {
    match source {
        app::picker::StorySource::Library(_) => app::picker::library_rows(dir, root, data_base),
        other @ app::picker::StorySource::DiskSet { .. } => other.scan(data_base),
    }
}

/// The first row that is a story, for the paths that must pick one without a
/// terminal to ask on. `None` when the folder holds only folders.
fn first_story(stories: &[app::picker::StoryEntry]) -> Option<&app::picker::StoryEntry> {
    stories.iter().find(|e| !e.is_folder())
}

/// Add to the in-memory index whatever stories in `rows` it does not hold yet
/// (a download landed in the folder on screen after the walk passed it).
fn merge_index(index: &mut Vec<app::picker::StoryEntry>, rows: &[app::picker::StoryEntry]) {
    for r in rows.iter().filter(|e| !e.is_folder()) {
        if !index.iter().any(|e| e.same_story(r)) {
            index.push(r.clone());
        }
    }
}

/// Replace the list with `dir := target`'s rows and realign the two per-index
/// caches, the same three moves the download drain makes. Going up lands the
/// selection on the folder just left; going down lands on the first row.
#[allow(clippy::too_many_arguments)]
fn enter_folder(
    source: &app::picker::StorySource,
    dir: &mut std::path::PathBuf,
    root: &std::path::Path,
    target: &std::path::Path,
    stories: &mut Vec<app::picker::StoryEntry>,
    row_badges: &mut Vec<app::picker::RowBadges>,
    aux_cache: &mut Vec<Option<app::picker::StoryAux>>,
    list: &mut app::list_scroll::ListScroll,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
    viewport: usize,
    anim: &app::config::AnimationConfig,
) {
    let came_from = std::mem::replace(dir, target.to_path_buf());
    *stories = rows_for(source, dir, root, data_base);
    *row_badges = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, hint_index))
        .collect();
    *aux_cache = (0..stories.len()).map(|_| None).collect();
    list.len(stories.len());
    let idx = stories.iter().position(|e| e.is_folder() && e.path == came_from).unwrap_or(0);
    list.select(idx, viewport, anim);
}

/// Replace the list with the index's matches for `query` and realign the
/// caches. The selection goes back to the top: the rows under it are new.
#[allow(clippy::too_many_arguments)]
fn apply_find(
    index: &[app::picker::StoryEntry],
    root: &std::path::Path,
    query: &str,
    stories: &mut Vec<app::picker::StoryEntry>,
    row_badges: &mut Vec<app::picker::RowBadges>,
    aux_cache: &mut Vec<Option<app::picker::StoryAux>>,
    list: &mut app::list_scroll::ListScroll,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) {
    *stories = app::picker::search_library(index, root, query);
    *row_badges = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, hint_index))
        .collect();
    *aux_cache = (0..stories.len()).map(|_| None).collect();
    list.len(stories.len());
    list.selected = 0;
}

/// Whether the list on screen is the gallery's recursive view: the cover grid,
/// no find field open, and a library index to draw from (a disk set has none,
/// and shows its rows as tiles as it always did).
fn gallery_all_folders(view: PickerView, finding: bool, has_index: bool) -> bool {
    matches!(view, PickerView::Gallery) && !finding && has_index
}

/// Replace the list with every story under `dir` (the gallery's view of a
/// folder), keeping the selection on the same story where it survives.
#[allow(clippy::too_many_arguments)]
fn show_gallery_scope(
    index: &[app::picker::StoryEntry],
    root: &std::path::Path,
    dir: &std::path::Path,
    stories: &mut Vec<app::picker::StoryEntry>,
    row_badges: &mut Vec<app::picker::RowBadges>,
    aux_cache: &mut Vec<Option<app::picker::StoryAux>>,
    list: &mut app::list_scroll::ListScroll,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) {
    let keep = stories.get(list.selected).filter(|e| !e.is_folder()).map(|e| (e.path.clone(), e.meta.disk_entry.clone()));
    *stories = app::picker::search_library_under(index, root, dir, "");
    *row_badges = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, hint_index))
        .collect();
    *aux_cache = (0..stories.len()).map(|_| None).collect();
    list.len(stories.len());
    list.selected = keep
        .and_then(|(p, d)| stories.iter().position(|e| e.is(&p, d.as_deref())))
        .unwrap_or(0);
}

/// What the picker's title line says, and which folder the row painter
/// measures a match's folder label against.
pub(crate) struct PickerHeading<'a> {
    /// The folder being listed: the root until the user descends.
    pub dir: &'a std::path::Path,
    /// The library root; a find match's folder label is relative to it.
    pub root: &'a std::path::Path,
    /// `Some` while find-story's field is open.
    pub find: Option<FindStatus<'a>>,
    /// The cover gallery, showing every story under `dir` rather than the
    /// folder's own rows (`None` in the list, and with no index to draw on).
    pub all_folders: Option<IndexStatus>,
}

/// How far the library index has got, for a header that draws on it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexStatus {
    pub indexed: usize,
    pub done: bool,
}

/// The find field's state, as the header reports it.
pub(crate) struct FindStatus<'a> {
    pub query: &'a str,
    /// Stories indexed so far, shown while the walk is still running.
    pub indexed: usize,
    pub done: bool,
}

impl<'a> PickerHeading<'a> {
    /// A folder view of `dir`, which is also the root.
    #[cfg(test)]
    fn browse(dir: &'a std::path::Path) -> Self {
        PickerHeading { dir, root: dir, find: None, all_folders: None }
    }

    /// The folder a row's label is relative to: the root while finding (a match
    /// can be anywhere under it), the listed folder otherwise (so no row in a
    /// folder view wears one).
    fn label_base(&self) -> &std::path::Path {
        if self.find.is_some() { self.root } else { self.dir }
    }

    /// The title line. `toggle` is the view-flip hint (`g: covers` / `g: list`).
    fn line(&self, stories: &[app::picker::StoryEntry], toggle: &str) -> String {
        match &self.find {
            Some(f) => {
                let n = stories.len();
                let es = if n == 1 { "" } else { "es" };
                let progress = if f.done { String::new() } else { format!(" · indexing, {} so far", f.indexed) };
                format!(
                    " lanthorn — find a story  ({n} match{es} for “{}” in {}{progress})   [i: info · {toggle}]",
                    f.query,
                    self.root.display()
                )
            }
            None if self.all_folders.is_some() => {
                let status = self.all_folders.expect("checked");
                let progress = if status.done { String::new() } else { format!(" · indexing, {} so far", status.indexed) };
                format!(
                    " lanthorn — choose a story  ({} in {} and its folders{progress})   [i: info · {toggle}]",
                    stories.len(),
                    self.dir.display()
                )
            }
            None => {
                let folders = stories.iter().filter(|e| e.is_folder() && e.title != app::picker::PARENT_LABEL).count();
                let n = stories.iter().filter(|e| !e.is_folder()).count();
                let f = match folders {
                    0 => String::new(),
                    1 => ", 1 folder".to_string(),
                    k => format!(", {k} folders"),
                };
                format!(" lanthorn — choose a story  ({n} found{f} in {})   [i: info · {toggle}]", self.dir.display())
            }
        }
    }
}

/// The info panel for a folder row: where it leads, and how. Returns the
/// panel's scroll extent, which is nothing.
fn draw_folder_panel(
    entry: &app::picker::StoryEntry,
    root: &std::path::Path,
    area: Rect,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
) -> usize {
    if area.width < 2 || area.height < 2 {
        return 0;
    }
    let story_info = cs.theme.get("story_info").style;
    let story_info_title = cs.theme.get("story_info_title").style;
    let story_info_value = cs.theme.get("story_info_value").style;
    let story_info_label = cs.theme.get("story_info_label").style;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(story_info);
            }
        }
    }
    let x = area.x + 1;
    let inner = Rect::new(x, area.y, area.width.saturating_sub(2), area.height);
    let is_parent = entry.title == app::picker::PARENT_LABEL;
    let title = if is_parent { "Up one folder".to_string() } else { entry.title.clone() };
    draw_str_clipped(buf, x, area.y + 1, &title, story_info_title, inner);
    let rel = entry.path.strip_prefix(root).ok().filter(|r| !r.as_os_str().is_empty());
    let where_ = match rel {
        Some(r) => format!("{}/", r.display()),
        None => "the library root".to_string(),
    };
    draw_str_clipped(buf, x, area.y + 3, "Leads to", story_info_label, inner);
    draw_str_clipped(buf, x, area.y + 4, &where_, story_info_value, inner);
    draw_str_clipped(buf, x, area.y + 6, "Enter opens it; Backspace goes up.", story_info_value, inner);
    0
}

/// Reorder `stories` by `sort`, keeping the selection on the same story (by
/// path — see `resort_preserving_selection`), and keep the per-index caches
/// (`row_badges`, `aux_cache`) aligned with the new order. Every reorder in
/// the picker loop — `s`, `d`, a header click, and a fetch sweep landing new
/// titles — routes through this one function so no caller can forget to
/// invalidate the caches.
#[allow(clippy::too_many_arguments)]
fn resort_list(
    stories: &mut [app::picker::StoryEntry],
    selected: usize,
    sort: app::picker::Sort,
    row_badges: &mut Vec<app::picker::RowBadges>,
    aux_cache: &mut Vec<Option<app::picker::StoryAux>>,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) -> usize {
    let new_idx = app::picker::resort_preserving_selection(stories, selected, sort);
    *row_badges = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, hint_index))
        .collect();
    *aux_cache = (0..stories.len()).map(|_| None).collect();
    new_idx
}

/// Overlay a transient status line (fetch progress) onto the list's footer
/// row, replacing the normal footer hint text while a message is active.
fn draw_progress_line(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    text: &str,
    style: ratatui::style::Style,
) {
    if area.height < 4 {
        return; // matches draw_story_picker's own too-small-for-a-footer guard
    }
    let y = area.bottom().saturating_sub(1);
    for x in area.left()..area.right() {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol(" ").set_style(style);
        }
    }
    draw_str_clipped(buf, area.x, y, text, style, area);
}

/// Build the ratatui-image picker for cover art per the CLI mode. `Auto`
/// queries the terminal (falling back to half-blocks); forced modes query for
/// font size then pin the protocol. Returns `None` only if construction fails.
pub(crate) fn build_cover_picker(mode: app::config::ImageProtocol) -> Option<ratatui_image::picker::Picker> {
    use app::config::ImageProtocol as M;
    use ratatui_image::picker::{Picker, ProtocolType};
    match mode {
        M::Halfblocks => Some(Picker::halfblocks()),
        M::Auto => Some(Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())),
        M::Kitty | M::Sixel | M::Iterm2 => {
            let mut p = Picker::from_query_stdio().ok()?;
            p.set_protocol_type(match mode {
                M::Kitty => ProtocolType::Kitty,
                M::Sixel => ProtocolType::Sixel,
                M::Iterm2 => ProtocolType::Iterm2,
                _ => unreachable!(),
            });
            Some(p)
        }
    }
}

/// The terminal's cell size in pixels **right now**, from `TIOCGWINSZ`.
///
/// One `ioctl` on the tty: no escape written, no stdin read, nothing for the
/// app's own input loop to race with. That is the whole reason this exists
/// beside [`build_cover_picker`], whose `Picker::from_query_stdio` writes a
/// capability query and reads the reply — genuinely delicate mid-session with
/// the app in raw mode owning the keyboard, and the reason the cell size used to
/// be measured once at launch and never again (SQ-0988).
///
/// `None` when the terminal reports no pixel geometry (`ws_xpixel`/`ws_ypixel`
/// are documented as "unused" by the tty ioctl and are zero on plenty of
/// terminals, and Windows has no equivalent at all). A caller must then KEEP the
/// value it has: a default would be a guess replacing a measurement.
pub(crate) fn terminal_cell_size() -> Option<ratatui_image::FontSize> {
    let ws = crossterm::terminal::window_size().ok()?;
    if ws.width == 0 || ws.height == 0 || ws.columns == 0 || ws.rows == 0 {
        return None;
    }
    Some(ratatui_image::FontSize::new(ws.width / ws.columns, ws.height / ws.rows))
}

/// Re-derive `picker`'s cell size after a resize. Answers whether it MOVED, so
/// the caller can throw away what it fitted against the old one.
///
/// **The absolute size does not matter; the aspect ratio does.** Geometry
/// multiplies by `fw`/`fh` to reach a device box and divides by them again to
/// return to cells, so a uniform scale error cancels out. What survives is
/// `fw : fh` — and a cell is `round(advance_em · px)` by `round(line_em · px)`,
/// two roundings at different rates, so even a face whose design ratio is
/// exactly 2.002 (FiraCode) yields real cells from 1.750 (4x7 at 6 px) to 2.250
/// (4x9 at 7 px). Change font size mid-session and the composite is fitted with
/// an aspect up to ~29% wrong until the app is restarted; the art looks subtly
/// stretched and comes right again after a relaunch, which is exactly how it was
/// reported.
///
/// **The cell is the only thing that moved, so the cell is the only thing
/// touched.** The picker is mutated in place rather than rebuilt, because a
/// queried picker knows things this function cannot re-derive without asking the
/// terminal again: the protocol, and behind it the whole capability list —
/// `KittyCompression` (`o=z`, worth up to 88x on a raster composite),
/// `RectangularOps`, the tmux flag. A font change tells you nothing about any of
/// them.
///
/// This used to rebuild with the deprecated `Picker::from_fontsize` and copy the
/// protocol across by hand, which preserved exactly the one field it named:
/// `from_fontsize` constructs `capabilities: Vec::new()`, so a mid-session font
/// change silently dropped compression back to raw and left it there until the
/// app was relaunched. It fails safe, which is why nobody saw it (SQ-0992).
/// Re-querying is not the alternative either — `Picker::from_query_stdio` writes
/// an escape and reads the reply, which is the whole thing
/// [`terminal_cell_size`] exists to avoid.
pub(crate) fn refresh_cell_size(picker: &mut ratatui_image::picker::Picker) -> bool {
    let Some(fs) = terminal_cell_size() else { return false };
    apply_cell_size(picker, fs)
}

/// [`refresh_cell_size`] with the measurement handed in, so it can be driven
/// without a tty.
fn apply_cell_size(picker: &mut ratatui_image::picker::Picker, fs: ratatui_image::FontSize) -> bool {
    let was = picker.font_size();
    if (fs.width, fs.height) == (was.width, was.height) {
        return false;
    }
    picker.set_font_size(fs);
    true
}

/// What the browser hands back: the story to play, and the boot-time overrides
/// the user asked for on the way out (SQ-0789). `overrides` is empty for every
/// ordinary launch — Enter and a double left-click never touch it — so the
/// common path is byte-for-byte what it was.
pub(crate) struct PickedStory {
    pub path: std::path::PathBuf,
    /// Which story on `path`, when it is a disk image holding several
    /// (SQ-0859). `None` — every loose file, every single-story floppy — opens
    /// by path exactly as it always did.
    pub disk_entry: Option<String>,
    pub overrides: app::launch_options::LaunchOverrides,
}

impl PickedStory {
    /// Play the story one browser row stands for, with no overrides: its path
    /// **and** which story on the image it is.
    fn row(entry: &app::picker::StoryEntry) -> PickedStory {
        PickedStory {
            path: entry.path.clone(),
            disk_entry: entry.meta.disk_entry.clone(),
            overrides: app::launch_options::LaunchOverrides::default(),
        }
    }
}

/// Open the launch-options dialog for one browser row.
///
/// **The single seam both gestures go through.** Shift-Enter and the double
/// right-click both call exactly this, so the keyboard and the mouse cannot grow
/// separate ideas of what the dialog is seeded with — the drift this function
/// exists to prevent.
fn open_launch_options(
    entry: &app::picker::StoryEntry,
    cfg: &app::config::Config,
    data_base: &std::path::Path,
) -> app::launch_options::LaunchOptionsState {
    let game_dir = entry.game_dir(data_base);
    // What this story already inherits, which is what every "did the user change
    // it?" comparison is against.
    let inherited_pictures = app::styles::read_per_game_pictures(&game_dir);
    let inherited_interpreter =
        app::styles::read_per_game_interpreter_number(&game_dir).or(cfg.interpreter_number);
    // The Z-machine version, for the default half of the derived interpreter
    // number (6 for Version 6, else 1). A Glulx or Scott story has no header
    // 0x1E at all and the dialog says so rather than inventing a number.
    let z_version = matches!(entry.meta.engine, app::picker::Engine::ZCode)
        .then(|| entry.meta.version.as_deref().and_then(|v| v.parse::<u8>().ok()))
        .flatten();
    app::launch_options::LaunchOptionsState::new(
        &entry.title,
        &entry.path,
        inherited_pictures.as_deref(),
        inherited_interpreter,
        z_version,
        entry.meta.disk_image,
    )
    .on_disk_entry(entry.meta.disk_entry.as_deref())
}

/// Where one wheel notch over the picker goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WheelTarget {
    /// Swallowed by the topmost modal. The launch-options dialog's list of
    /// options is shorter than its own dialog, so under SQ-0831's rule there
    /// is nothing there to scroll — but the notch must still stop here rather
    /// than reaching the story list underneath, which would otherwise slide
    /// around behind an open modal (SQ-0832). The key reference and the
    /// per-story menu are the same case (SQ-1227): both are short, neither
    /// scrolls, and the list must not move behind either.
    Swallowed,
    /// The IFDB search modal's own results/files list.
    Search,
    /// The preview modal zooms instead of scrolling (SQ-0486).
    PreviewZoom,
    /// The info panel's body, when the pointer is over it.
    InfoPanel,
    /// The story list (or the cover grid) itself — the coalesced notch.
    StoryList,
}

/// Route a wheel notch through the picker's modal ladder. The order is
/// z-order, the same precedence the picker's clicks already take, and it lives
/// here as one total function so "which surface owns the wheel right now?" has
/// a single answer that can be pinned by a test.
fn wheel_target(
    launch_open: bool,
    keys_open: bool,
    menu_open: bool,
    search_open: bool,
    preview_open: bool,
    over_info_panel: bool,
) -> WheelTarget {
    if launch_open || keys_open || menu_open {
        WheelTarget::Swallowed
    } else if search_open {
        WheelTarget::Search
    } else if preview_open {
        WheelTarget::PreviewZoom
    } else if over_info_panel {
        WheelTarget::InfoPanel
    } else {
        WheelTarget::StoryList
    }
}

/// What a right-click on the story list does (SQ-1227), given the row it landed
/// on and whether that row is a folder: `(row to select, menu to open)`.
///
/// A total function of ONE click, deliberately. The gesture it replaced was a
/// double right-click with a 400ms recogniser and a tracker of its own
/// (SQ-0789), which nothing on screen mentioned and nobody found; a single click
/// needs no state, so there is none to get wrong. A folder is selected like any
/// row and has no menu — none of the items apply to it.
fn right_click_action(hit: Option<(usize, bool)>) -> (Option<usize>, Option<usize>) {
    match hit {
        Some((idx, false)) => (Some(idx), Some(idx)),
        Some((idx, true)) => (Some(idx), None),
        None => (None, None),
    }
}

/// Run the pre-game story picker over a [`app::picker::StorySource`] — a
/// directory passed at launch, or the multi-disk release one named volume
/// belongs to (SQ-0844). Returns the chosen story (with any launch-time
/// overrides), or `None` if the user quit. Exits the process with a message when
/// the source offers no launchable stories.
pub(crate) fn run_story_picker(
    mut source: app::picker::StorySource,
    cfg: &app::config::Config,
    data_base: &std::path::Path,
) -> Option<PickedStory> {
    // The library root, and the folder currently listed. They part company the
    // moment the user descends into a sub-folder (Enter on a folder row) and
    // meet again on Backspace; downloads land in `dir`, the folder on screen.
    let root = source.dir().to_path_buf();
    let mut dir = root.clone();
    let mut stories = rows_for(&source, &dir, &root, data_base);
    if stories.is_empty() {
        eprintln!("lanthorn: no Z-machine story files found in '{}'", dir.display());
        std::process::exit(1);
    }

    // Resolve themed colors the same way the game does, so the picker matches.
    let (base, _w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (cs, _set, _w2) = app::style::resolve(&base, &cfg.user_dir);

    // Row badges: each story's per-game dir under `data_base` + one shared hint
    // index, computed once (SQ-0284). Recomputed by `resort_list` whenever the
    // list reorders, so it stays index-aligned with `stories`.
    let hint_index = app::hints::load_hint_index(&cfg.user_dir);
    let mut row_badges: Vec<app::picker::RowBadges> = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, &hint_index))
        .collect();
    let sym_cfg = app::style::finalize_symbols(&base.symbols);
    let badge_glyphs = app::picker::BadgeGlyphs::from_symbols(&sym_cfg);

    // Terminal setup mirrors the game loop. If any step fails we can't be
    // interactive — fall back to the first story rather than abort.
    if enable_raw_mode().is_err() {
        return first_story(&stories).map(PickedStory::row);
    }
    if execute!(stdout(), EnterAlternateScreen).is_err() {
        let _ = disable_raw_mode();
        return first_story(&stories).map(PickedStory::row);
    }
    // Mouse capture is opt-in (config `mouse = true`): its any-motion reporting
    // floods this loop with redraws on every mouse move. Off by default keeps the
    // browser snappy; click-to-select and wheel scroll require enabling it.
    if cfg.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(_) => {
            restore_terminal();
            return first_story(&stories).map(PickedStory::row);
        }
    };

    // `mut` since SQ-0988: a resize can move the terminal's cell size, and the
    // picker is re-derived from `TIOCGWINSZ` when it does.
    let mut cover_picker = if cfg.images { build_cover_picker(cfg.image_protocol) } else { None };
    let mut cover = app::cover::CoverState::default();

    // The browser's keys, resolved the same way the game's are (SQ-0796): the
    // built-in `Context::Browser` bindings with any `[keymap.browser]` overrides
    // layered on. Resolution warnings are dropped here on purpose — the game's
    // own startup resolves the very same config and reports them once.
    let (keymap, _keymap_warnings) = app::keymap::KeyMap::resolve(&cfg.keymap);

    let mut list = app::list_scroll::ListScroll::new();
    list.len(stories.len());
    let anim = &cfg.animation;
    let mut row_rects: Vec<(usize, Rect)> = Vec::new();
    let mut header_rects: Vec<(app::picker::SortKey, Rect)> = Vec::new();
    let mut viewport: usize = 0;
    let mut sort = app::picker::Sort::default();

    // Cover-gallery view state (SQ-0374): `view` selects list-vs-grid; the rest
    // is grid geometry from the last gallery draw, read by input handling
    // (2D navigation, paging) and the per-frame visible-tile cover requests.
    let mut view = PickerView::List;
    let mut gallery_first_row: usize = 0;
    let mut gallery_cols: usize = 1;
    let mut gallery_vis: usize = 1;
    let mut gallery_visible: Vec<usize> = Vec::new();
    // When the grid's scroll last moved — wheel or a nav key, mirroring
    // `AppState::sixel_scroll_motion_at` (SQ-1198) — this loop has no `AppState`
    // to ride, so it tracks the same debounce window locally. `None` = never
    // scrolled this session. See `gallery_scroll_in_motion` below.
    let mut gallery_scroll_motion_at: Option<std::time::Instant> = None;

    // IFDB fetch worker (SQ-0348): `f` (this story, forced) and `r` (whole
    // library, skip current-version) share one background worker. Live only
    // while this loop runs — dropping `fetcher` at the end drops its request
    // sender, which ends the worker thread's `recv()` loop.
    let fetcher = app::fetch_worker::Fetcher::new(
        Box::new(app::ifdb::IfdbClient::new()),
        data_base.to_path_buf(),
        Duration::from_millis(500),
    );
    // On-demand InvisiClues downloader (SQ-0445): `H` fetches a matching hint
    // file for the selected story when it has none locally. Shares the picker's
    // non-blocking drain-per-frame model with the IFDB fetcher.
    let mut hint_dl = app::hint_download::HintDownloader::new();

    // The footer-row status line while a fetch is in flight (or just finished);
    // `None` shows the normal footer hints instead.
    let mut progress_line: Option<String> = None;
    // True for an `f` order (single story, forced) — controls the completion
    // message's shape (`f`: found/not-found/failed; `r`: a tallied summary).
    let mut fetch_is_single = false;
    let (mut sweep_fetched, mut sweep_skipped, mut sweep_not_found, mut sweep_failed) = (0u32, 0u32, 0u32, 0u32);
    // Manual IFDB-page entry (SQ-0371): `Some` while the user is typing an IFDB
    // URL/id for the selected story; keystrokes route to the field, Enter
    // submits a fetch-by-id, Esc cancels.
    let mut manual_ifdb: Option<app::text_field::TextField> = None;

    // Open-a-URL (SQ-1086): `Some` while the user is typing an address. The
    // download runs on its own short-lived thread and lands in `dir` — the
    // library — because in the browser there is nothing to decide: this IS the
    // directory the picker reads, so a story fetched here is kept by definition
    // and there is no keep-it prompt to raise. (The command line, which has no
    // library in hand, is where that question gets asked.)
    let mut url_prompt: Option<app::text_field::TextField> = None;
    let mut url_dl = app::story_url::UrlDownloader::new();

    // Type-to-find over the WHOLE library (find-story). The field is `Some`
    // while the list shows matches instead of a folder; Esc puts the folder
    // back. What it matches against is an in-memory index of every story under
    // `root`, built once per picker on its own thread, one folder per batch,
    // because a scan opens every file it lists and a whole library is
    // gigabytes: the folder view is up in one directory's time, and the index
    // catches up behind it (the header says so until it has).
    let mut find_field: Option<app::text_field::TextField> = None;
    let index_rx = match &source {
        app::picker::StorySource::Library(_) => {
            Some(app::picker::spawn_library_index(root.clone(), data_base.to_path_buf()))
        }
        // A multi-disk set is one release, not a tree; there is nothing to walk.
        app::picker::StorySource::DiskSet { .. } => None,
    };
    let mut index: Vec<app::picker::StoryEntry> = Vec::new();
    let mut index_done = index_rx.is_none();

    // IFDB story search (SQ-0413): `/` opens a modal to search IFDB, browse
    // results, and download a chosen story file into `dir`. Network runs on its
    // own serial worker (one request at a time), drained per frame like the
    // fetcher; the modal is `Some` while open. The picker is always launched on
    // a directory (never a single file), so `dir` is always a valid download
    // target and the entry point is always available.
    //
    // The worker also holds an `IfdbClient` as its `MetadataSource` (SQ-0474):
    // after a download, it reuses that client's `fetch_cover` — never
    // `fetch`/`fetch_by_id` — to populate the story's sidecar + cover from the
    // iFiction record already resolved for the download, with zero extra
    // metadata requests. See `ifdb_search.rs`'s module header.
    let search_worker = app::ifdb_search::SearchWorker::new(
        Box::new(app::ifdb_search::IfdbSearchClient::new()),
        Box::new(app::ifdb::IfdbClient::new()),
        data_base.to_path_buf(),
    );
    let mut search_modal: Option<app::ifdb_search_modal::SearchModal> = None;
    let mut search_area = Rect::new(0, 0, 0, 0);
    let mut search_close_rect: Option<Rect> = None;

    // Info panel: always starts closed each launch (session-only state).
    let mut slide = PanelSlide::closed();
    let mut aux_cache: Vec<Option<app::picker::StoryAux>> =
        (0..stories.len()).map(|_| None).collect();
    let mut last_area = Rect::new(0, 0, 0, 0);
    let mut last_panel_area = Rect::new(0, 0, 0, 0);
    let mut panel_scroll: usize = 0;
    let mut panel_max: usize = 0;
    // Screen rect + full URL of each OSC 8 link the info panel drew this frame,
    // so a click can open it despite mouse capture (SQ-0367). Empty while the
    // panel is closed (refilled only when draw_info_panel runs).
    let mut panel_link_rects: Vec<(Rect, String)> = Vec::new();
    // Screen rect + resource ref of each previewable Pict/Snd row drawn this
    // frame (SQ-0347), so a click opens its preview modal.
    let mut panel_resource_rects: Vec<(Rect, ResourceRef)> = Vec::new();

    // Resource-preview modal (SQ-0347): `Some` while a bundled image/sound is
    // shown over the picker. `audio` is constructed lazily on the first sound
    // play and held for the loop's lifetime (its OutputStream must outlive
    // playback; per-click construct/drop would cut the sound and stutter).
    let mut preview: Option<ResourcePreview> = None;
    let mut audio: Option<audio::AudioBackend> = None;
    let mut preview_close_rect: Option<Rect> = None;
    let mut preview_button_rects: Vec<(app::render::dialog::ButtonId, Rect)> = Vec::new();
    let mut preview_area = Rect::new(0, 0, 0, 0);

    // Async cover decode: a background worker decodes off the main loop; results
    // are drained into `cover` each iteration. `requested` tracks in-flight paths
    // (so we don't re-queue), and the settle-debounce below waits until a
    // selection has been stable before requesting — a fling costs one decode.
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::time::Instant;
    let decoder = app::cover::CoverDecoder::new();
    // Async gallery-tile ENCODE (SQ-1199), the second half of the same pipeline:
    // once a cover is decoded, fitting it to a tile box and encoding the
    // terminal protocol is heavier still, and used to run inside the draw. It
    // now runs on this worker; the draw enqueues and paints the letterbox.
    let mut tile_encoder = app::cover::TileEncoder::new();
    let mut requested: HashSet<PathBuf> = HashSet::new();
    let mut last_sel = usize::MAX;
    let mut sel_changed_at = Instant::now();
    const COVER_DEBOUNCE: Duration = Duration::from_millis(90);
    // Story-list clicks: first click selects, a second on the same row within
    // this window launches it (SQ-0366).
    let mut last_click: Option<(usize, Instant)> = None;
    const DOUBLE_CLICK: Duration = Duration::from_millis(400);

    // The per-story menu (SQ-1227): `Some` while it is open over the list or the
    // gallery. Opened by `Space` or a SINGLE right-click on a row — which is
    // what replaced SQ-0789's double-right-click shortcut to the launch-options
    // dialog: the same dialog is one item down this menu, where it can be SEEN
    // rather than guessed at.
    let mut story_menu: Option<app::story_menu::StoryMenu> = None;
    let mut menu_rects: Vec<(usize, Rect)> = Vec::new();
    let mut menu_area = Rect::new(0, 0, 0, 0);
    // The browser's key reference (`?`, SQ-1227) — its own dialog, since the
    // game's hotkey panel is fed from an `AppState` this loop does not have.
    let mut keys_dialog = false;
    let mut keys_close_rect: Option<Rect> = None;
    let mut keys_button_rects: Vec<(app::render::dialog::ButtonId, Rect)> = Vec::new();
    let mut keys_area = Rect::new(0, 0, 0, 0);

    // Launch-options dialog (SQ-0789): `Some` while open, over the browser.
    // Opened only on an explicit gesture, so a plain launch never meets it.
    let mut launch_opts: Option<app::launch_options::LaunchOptionsState> = None;
    let mut launch_area = Rect::new(0, 0, 0, 0);
    let mut launch_close_rect: Option<Rect> = None;
    let mut launch_button_rects: Vec<(app::render::dialog::ButtonId, Rect)> = Vec::new();
    let mut launch_row_rects: Vec<(usize, Rect)> = Vec::new();
    // A failed sidecar write, reported once the alternate screen is gone.
    let mut persist_error: Option<String> = None;
    // A physical wheel notch emits several events, all delivered to the input
    // buffer together. Record the direction here and apply exactly one scroll
    // step once the buffer drains (at the loop top), so one notch = one row
    // (one grid row in the gallery) regardless of how the terminal spaces the
    // events within a notch.
    let mut pending_wheel: Option<isize> = None;

    let chosen: Option<PickedStory> = loop {
        // Restore the terminal + exit if an external termination signal arrived.
        exit_if_terminated();

        // Apply a coalesced wheel step once its notch's event burst has fully
        // drained from the input buffer (poll(0) empty). Separate notches are not
        // buffered together, so each still scrolls exactly one row.
        if let Some(d) = pending_wheel {
            if !crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
                pending_wheel = None;
                let before = list.selected;
                if matches!(view, PickerView::Gallery) {
                    // One notch = one grid row (a whole row of tiles) of SCROLL;
                    // the selection is clamped into the visible grid (SQ-0831).
                    let (fr, ni) = app::cover_gallery::wheel_scroll(
                        gallery_first_row, list.selected, gallery_cols, gallery_vis,
                        stories.len(), d,
                    );
                    gallery_first_row = fr;
                    list.select(ni, viewport, anim);
                    gallery_scroll_motion_at = Some(Instant::now());
                } else {
                    list.scroll_by(d, viewport, anim);
                }
                // Only a changed story invalidates the info panel's scroll.
                if list.selected != before {
                    panel_scroll = 0;
                }
            }
        }

        let _ = terminal.draw(|f| {
            let area = f.area();
            last_area = area;
            let buf = f.buffer_mut();
            let (list_area, panel_area) = split_picker_area(area, slide.fraction());
            let heading = PickerHeading {
                dir: &dir,
                root: &root,
                find: find_field.as_ref().map(|f| FindStatus { query: f.as_str(), indexed: index.len(), done: index_done }),
                all_folders: gallery_all_folders(view, find_field.is_some(), index_rx.is_some())
                    .then_some(IndexStatus { indexed: index.len(), done: index_done }),
            };
            match view {
                PickerView::List => {
                    let (rects, vp, hrects) = draw_story_picker(
                        &stories, &list, &row_badges, &badge_glyphs, &heading, &cs, &keymap,
                        sort, list_area, buf,
                    );
                    row_rects = rects;
                    viewport = vp;
                    header_rects = hrects;
                }
                PickerView::Gallery => {
                    // Skip the gallery while the resource-preview modal is open: its
                    // selected-cover background fill and cover images would otherwise
                    // render behind the dialog, bleeding the selection colour into it
                    // and corrupting its border where covers meet the edges (SQ-0389).
                    if preview.is_none() {
                        let (rects, cols, vis) = draw_story_gallery(
                            &stories, list.selected, &mut gallery_first_row, &heading, &cs, &keymap,
                            cover_picker.as_ref(), gallery_scroll_in_motion(gallery_scroll_motion_at),
                            &mut cover, &mut tile_encoder, data_base, list_area, buf,
                        );
                        gallery_cols = cols.max(1);
                        gallery_vis = vis.max(1);
                        gallery_visible = rects.iter().map(|(i, _)| *i).collect();
                        // A viewport analogue for ListScroll's easing while it holds
                        // the shared selection; the grid does its own scrolling.
                        viewport = (cols * vis).max(1);
                        row_rects = rects;
                        header_rects = Vec::new();
                    }
                }
            }
            // The manual IFDB-entry prompt (SQ-0371) takes the footer row while
            // active; otherwise a fetch's status line, otherwise the hints.
            let story_header_active = cs.theme.get("story_header_active").style;
            if let Some(field) = &find_field {
                let prompt = format!(
                    "Find (type to filter, ↑/↓ choose, Enter opens, Esc back to the folder): {}\u{258f}",
                    field.as_str()
                );
                draw_progress_line(buf, list_area, &prompt, story_header_active);
            } else if let Some(field) = &manual_ifdb {
                let prompt = format!("IFDB URL or id (Enter to fetch, Esc to cancel): {}▏", field.as_str());
                draw_progress_line(buf, list_area, &prompt, story_header_active);
            } else if let Some(field) = &url_prompt {
                let prompt = format!(
                    "Story URL (Enter to download, Esc to cancel): {}\u{258f}",
                    field.as_str()
                );
                draw_progress_line(buf, list_area, &prompt, story_header_active);
            } else if let Some(msg) = &progress_line {
                draw_progress_line(buf, list_area, msg, story_header_active);
            }

            // The per-story menu (SQ-1227), over the list and never over the
            // footer row — the footer is what says the menu key exists, so the
            // menu covering it would hide its own instructions. Anchored on the
            // highlighted row when that row is on screen; centred on the pane
            // when it is not (a fetch can resort the list under an open menu).
            if let Some(menu) = &story_menu {
                let pane = Rect::new(
                    list_area.x,
                    list_area.y,
                    list_area.width,
                    list_area.height.saturating_sub(1),
                );
                let anchor = row_rects
                    .iter()
                    .find(|(i, _)| *i == menu.story)
                    .map(|(_, r)| *r)
                    .unwrap_or_else(|| Rect::new(pane.x, pane.y, pane.width, 1));
                let rects =
                    app::story_menu::draw_story_menu(menu, anchor, pane, &keymap, &cs, buf);
                menu_area = rects.area;
                menu_rects = rects.items;
            }
            if preview.is_none() && panel_area.width > 0 {
                if let Some(entry) = stories.get(list.selected).filter(|e| e.is_folder()) {
                    last_panel_area = panel_area;
                    panel_link_rects.clear();
                    panel_resource_rects.clear();
                    panel_max = draw_folder_panel(entry, &root, panel_area, &cs, buf);
                } else if let Some(entry) = stories.get(list.selected) {
                    last_panel_area = panel_area;
                    panel_max = draw_info_panel(
                        &entry.title,
                        &entry.filename,
                        &entry.meta,
                        aux_cache[list.selected].as_ref(),
                        panel_scroll,
                        panel_area,
                        cover_picker.as_ref(),
                        &mut cover,
                        &entry.path,
                        &entry.cover_key(data_base),
                        slide.active(),
                        entry.hint_sidecar.as_deref(),
                        &cs,
                        buf,
                        &mut panel_link_rects,
                        &mut panel_resource_rects,
                    );
                } else {
                    panel_link_rects.clear();
                    panel_resource_rects.clear();
                }
            } else {
                // No selectable story, or the resource-preview modal is open: skip
                // redrawing the info panel. While the modal is up this stops the
                // panel's IFDB OSC-8 hyperlink from bleeding across the dialog
                // (SQ-0389), and drops its now-hidden click rects.
                panel_link_rects.clear();
                panel_resource_rects.clear();
            }

            // Resource-preview modal (SQ-0347): drawn last, over everything.
            if let Some(pv) = &mut preview {
                let rects = draw_resource_preview(pv, area, cover_picker.as_ref(), &cs, buf, &mut cover);
                preview_area = rects.area;
                preview_close_rect = rects.close;
                preview_button_rects = rects.buttons;
            }

            // IFDB search modal (SQ-0413): also drawn last (never with the
            // preview open — the two entry points are mutually exclusive).
            if let Some(sm) = &mut search_modal {
                let rects = app::ifdb_search_modal::draw_search_modal(sm, area, &cs, buf);
                search_area = rects.area;
                search_close_rect = rects.close;
            }

            // Launch-options dialog (SQ-0789): topmost of all, since it is the
            // last thing between the user and a booting story.
            if let Some(lo) = &launch_opts {
                if let Some(rects) =
                    app::render::launch_options_dialog::draw_launch_options(lo, area, &cs, buf)
                {
                    launch_area = rects.area;
                    launch_close_rect = rects.close;
                    launch_button_rects = rects.buttons;
                    launch_row_rects = rects.rows;
                }
            }

            // The key reference (SQ-1227): topmost of all, since it is the one
            // surface a lost user reaches for.
            if keys_dialog {
                if let Some(rects) =
                    app::render::browser_keys::draw_browser_keys(&keymap, area, &cs, buf)
                {
                    keys_area = rects.area;
                    keys_close_rect = rects.close;
                    keys_button_rects = rects.buttons;
                }
            }

            // Free any kitty uploads the cover/tile caches abandoned since the
            // last frame (SQ-1190) — this loop has no `GraphicsRender` of its
            // own, so `cover` keeps and flushes its own queue the same way.
            cover.flush_kitty_deletes(area, buf);
        });

        // Housekeeping (runs every iteration, before the poll gate below, so a
        // timed-out tick still drains results and re-issues the debounced request).
        // Drain finished decodes into the multi-entry cache.
        let mut cover_arrived = false;
        for (path, img) in decoder.drain() {
            cover.insert(path.clone(), img);
            requested.remove(&path);
            cover_arrived = true;
        }
        // Drain finished tile encodes into the tile cache (SQ-1199). A raster
        // fitted against a cell the terminal no longer has is dropped rather
        // than cached (`insert_tile`) — the request was in flight when the font
        // size moved and `invalidate_cell_geometry` threw the rest away.
        {
            let cell = cover_picker
                .as_ref()
                .map_or((0, 0), |p| (p.font_size().width, p.font_size().height));
            for (key, proto) in tile_encoder.drain() {
                if let Some(p) = proto {
                    cover.insert_tile(key, p, cell);
                }
                cover_arrived = true;
            }
        }
        // `.get`, not indexing (SQ-0659): `stories` can be empty — e.g. a
        // post-download rescan of a directory whose files all vanished.
        // The cover is asked for by the ROW's key, not by its path: one image can
        // be five rows, and each of them has its own jacket (SQ-0859).
        if let Some((sel, sel_game_dir)) = slide
            .open
            .then(|| {
                stories
                    .get(list.selected)
                    .filter(|e| !e.is_folder())
                    .map(|e| (e.cover_key(data_base), e.game_dir(data_base)))
            })
            .flatten()
        {
            ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
            if list.selected != last_sel {
                last_sel = list.selected;
                sel_changed_at = Instant::now();
            }
            // Settle-debounce: only request once the selection has been stable, so a
            // fling through the list costs one decode instead of one per row.
            if app::cover::should_request_cover(
                cover.has(&sel),
                requested.contains(&sel),
                sel_changed_at.elapsed(),
                COVER_DEBOUNCE,
            ) {
                decoder.request(sel.clone(), sel_game_dir);
                requested.insert(sel);
            }
        }
        // Gallery view (SQ-0374): decode every visible tile's cover, not just the
        // selection — the whole grid shows art. No settle-debounce here: the user
        // wants them all, and the worker decodes them one at a time as it can.
        if view == PickerView::Gallery {
            for &idx in &gallery_visible {
                if let Some(entry) = stories.get(idx).filter(|e| !e.is_folder()) {
                    let p = entry.cover_key(data_base);
                    if !cover.has(&p) && !requested.contains(&p) {
                        decoder.request(p.clone(), entry.game_dir(data_base));
                        requested.insert(p);
                    }
                }
            }
        }

        // Drain fetch progress (SQ-0348): each completed story's sidecar may
        // have just been (re)written, so re-resolve its entry in place — same
        // path both `f` and `r` take, since a single-story order is just an
        // order of length one — then re-sort through the one shared helper so
        // the cursor stays on whatever story the user is actually looking at,
        // not wherever its index happened to land.
        // The library index arrives one folder at a time; a disconnected
        // channel is the walk finishing. While the find field is open, each
        // arrival widens the match list in place, so a query typed two seconds
        // after launch still ends up seeing the whole library.
        let mut index_grew = false;
        if !index_done {
            if let Some(rx) = index_rx.as_ref() {
                loop {
                    match rx.try_recv() {
                        Ok(batch) => {
                            index.extend(batch.entries);
                            index_grew = true;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            index_done = true;
                            break;
                        }
                    }
                }
            }
        }
        if index_grew {
            if let Some(field) = &find_field {
                apply_find(&index, &root, field.as_str(), &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
            } else if gallery_all_folders(view, false, index_rx.is_some()) {
                show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
            }
        }

        let mut fetch_arrived = false;
        for p in fetcher.drain() {
            fetch_arrived = true;
            match &p.outcome {
                app::fetch_worker::Outcome::Fetched => sweep_fetched += 1,
                app::fetch_worker::Outcome::Skipped => sweep_skipped += 1,
                app::fetch_worker::Outcome::NotFound => sweep_not_found += 1,
                app::fetch_worker::Outcome::Failed(_) => sweep_failed += 1,
            }
            // Only Fetched/NotFound actually (re)write the sidecar (a Skipped
            // story's cache was already current; a Failed story's write is
            // withheld so a later `r` retries it) — no point re-reading disk
            // for the other two.
            let rewrote_sidecar =
                matches!(p.outcome, app::fetch_worker::Outcome::Fetched | app::fetch_worker::Outcome::NotFound);
            if rewrote_sidecar {
                // By ROW, not by path: a compilation contributes several rows
                // that share one path, and only the disk entry says which of
                // them this result belongs to (SQ-0859).
                let disk_entry = p.disk_entry.as_deref();
                if let Some(fresh) = app::picker::resolve_entry_from(&p.path, disk_entry, data_base)
                {
                    if let Some(slot) = stories.iter_mut().find(|e| e.is(&p.path, disk_entry)) {
                        *slot = fresh;
                    }
                }
                // A fetch may have just written a cover.png; drop any cached
                // "coverless" decode so the panel re-reads and shows it now,
                // rather than only after the picker is reopened.
                if matches!(p.outcome, app::fetch_worker::Outcome::Fetched) {
                    if let Some(key) = stories
                        .iter()
                        .find(|e| e.is(&p.path, disk_entry))
                        .map(|e| e.cover_key(data_base))
                    {
                        cover.forget(&key);
                        requested.remove(&key);
                    }
                }
            }
            progress_line = Some(if fetch_is_single {
                match &p.outcome {
                    app::fetch_worker::Outcome::Fetched => format!("Fetched {}", p.title),
                    app::fetch_worker::Outcome::Skipped => format!("Fetched {}", p.title),
                    app::fetch_worker::Outcome::NotFound => format!("No IFDB record for {}", p.title),
                    app::fetch_worker::Outcome::Failed(reason) => format!("Fetch failed: {reason}"),
                }
            } else if p.done < p.total {
                format!("Fetching {}/{} — {}", p.done, p.total, p.title)
            } else {
                let mut msg = format!(
                    "Fetched {}, skipped {}, not found {}",
                    sweep_fetched, sweep_skipped, sweep_not_found
                );
                if sweep_failed > 0 {
                    msg.push_str(&format!(", failed {sweep_failed}"));
                }
                msg
            });
        }
        if fetch_arrived {
            // A fetch just rewrote titles and authors in `stories`; the index
            // holds its own copies, and find matches on those.
            for e in stories.iter().filter(|e| !e.is_folder()) {
                if let Some(slot) = index.iter_mut().find(|i| i.same_story(e)) {
                    *slot = e.clone();
                }
            }
            list.select(
                resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                viewport,
                anim,
            );
        }

        // Drain hint downloads (SQ-0445): a completed one wrote a sidecar beside
        // the story, so mark that entry as now having a hint and relight its
        // badge in place. The file isn't a list row (it was never scanned), so
        // there's nothing to hide this session — a later relaunch's `scan_stories`
        // hides + associates it like any other sidecar.
        let mut hint_arrived = false;
        for r in hint_dl.drain() {
            hint_arrived = true;
            match r.outcome {
                app::hint_download::HintDlOutcome::Done => {
                    if let Some(idx) =
                        stories.iter().position(|e| e.is(&r.story, r.disk_entry.as_deref()))
                    {
                        stories[idx].hint_sidecar = Some(r.dest);
                        row_badges[idx] = app::picker::compute_row_badges(&stories[idx], data_base, &hint_index);
                    }
                    progress_line = Some(format!("Downloaded hints for {}", r.title));
                }
                app::hint_download::HintDlOutcome::Failed(msg) => {
                    progress_line = Some(format!("Hint download failed: {msg}"));
                }
            }
        }

        // Drain the open-a-URL downloads (SQ-1086). A finished one lands in
        // `dir`, so the library has to be rescanned and the cursor put on the new
        // story — the same landing the IFDB download below performs, through the
        // same `ifdb_download_landing` seam, so the two gestures cannot end
        // anywhere different.
        let mut url_arrived = false;
        for r in url_dl.drain() {
            url_arrived = true;
            match r.outcome {
                Ok(new_path) => {
                    let name =
                        new_path.file_name().and_then(|n| n.to_str()).unwrap_or("story").to_string();
                    let prev_row = stories
                        .get(list.selected)
                        .map(|e| (e.path.clone(), e.meta.disk_entry.clone()));
                    // A set browser scans its volumes, not a directory, so a
                    // fetched story would otherwise land on disk and vanish from
                    // the list that ordered it — exactly SQ-0844's case.
                    if let app::picker::StorySource::DiskSet { members, .. } = &mut source {
                        if !members.contains(&new_path) {
                            members.push(new_path.clone());
                        }
                    }
                    stories = rows_for(&source, &dir, &root, data_base);
                    merge_index(&mut index, &stories);
                    if gallery_all_folders(view, find_field.is_some(), index_rx.is_some()) {
                        show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                    }
                    app::picker::resort_preserving_selection(&mut stories, 0, sort);
                    row_badges = stories
                        .iter()
                        .map(|e| app::picker::compute_row_badges(e, data_base, &hint_index))
                        .collect();
                    aux_cache = (0..stories.len()).map(|_| None).collect();
                    list.len(stories.len());
                    let (idx, line) = ifdb_download_landing(
                        stories.iter().position(|e| e.path == new_path),
                        prev_row
                            .and_then(|(p, d)| stories.iter().position(|e| e.is(&p, d.as_deref()))),
                        stories.len(),
                        &name,
                    );
                    list.select(idx, viewport, anim);
                    progress_line = Some(line);
                }
                // Say what was fetched and why it could not be opened — a 404
                // page, a login redirect and a PDF are three different mistakes.
                Err(msg) => progress_line = Some(format!("Could not open {}: {msg}", r.url)),
            }
        }

        // Drain IFDB search worker events (SQ-0413). A downloaded file lands in
        // `dir`, so rescan the directory, honour the active sort, and land the
        // cursor on the new story; other events (search results, download
        // options, errors) update the modal's own state machine, which may hand
        // back a follow-up action.
        let mut search_arrived = false;
        for ev in search_worker.drain() {
            search_arrived = true;
            if let app::ifdb_search::SearchEvent::Downloaded(new_path) = &ev {
                let name = new_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("story")
                    .to_string();
                let prev_row = stories
                    .get(list.selected)
                    .map(|e| (e.path.clone(), e.meta.disk_entry.clone()));
                // A story downloaded into a *set* browser is not a volume of the
                // release, so the set's own scan would never show it. Adopt it
                // as one more path to read, which is all a member ever is —
                // otherwise the download lands on disk and vanishes from the
                // list that ordered it (SQ-0844).
                if let app::picker::StorySource::DiskSet { members, .. } = &mut source {
                    if !members.contains(new_path) {
                        members.push(new_path.clone());
                    }
                }
                stories = rows_for(&source, &dir, &root, data_base);
                    merge_index(&mut index, &stories);
                    if gallery_all_folders(view, find_field.is_some(), index_rx.is_some()) {
                        show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                    }
                app::picker::resort_preserving_selection(&mut stories, 0, sort);
                row_badges = stories
                    .iter()
                    .map(|e| app::picker::compute_row_badges(e, data_base, &hint_index))
                    .collect();
                aux_cache = (0..stories.len()).map(|_| None).collect();
                list.len(stories.len());
                let (idx, line) = ifdb_download_landing(
                    stories.iter().position(|e| &e.path == new_path),
                    prev_row.and_then(|(p, d)| {
                        stories.iter().position(|e| e.is(&p, d.as_deref()))
                    }),
                    stories.len(),
                    &name,
                );
                list.select(idx, viewport, anim);
                progress_line = Some(line);
                search_modal = None;
                continue;
            }
            if search_modal.is_some() {
                let action = search_modal.as_mut().unwrap().on_event(&ev);
                dispatch_search_action(action, &search_worker, &dir, &mut search_modal);
            }
        }

        // A decode just landed: loop back to redraw so the cover paints now. The
        // draw is at the top of the loop, and once the result is cached
        // `cover_busy` goes false — without this the loop would block on `read()`
        // and the new cover wouldn't appear until the next input event.
        // `index_grew` too (SQ-none): a find's matches and the gallery's scope
        // widen as folders are indexed, and a header that counts them must
        // repaint without waiting for a key.
        if cover_arrived || fetch_arrived || hint_arrived || search_arrived || url_arrived || index_grew {
            list.finalize_if_done();
            continue;
        }

        // Tick while a scroll or panel-slide animation eases so the motion is
        // visible, or while a cover decode is in flight / still needed, or a
        // fetch sweep is running, so results drain and redraw without a
        // keypress; otherwise block until the next event.
        let sel_now = stories.get(list.selected).map(|e| &e.path);
        let panel_busy = slide.open
            && sel_now.is_some_and(|p| !requested.is_empty() || !cover.has(p));
        // Gallery keeps ticking while any tile cover is still decoding — or,
        // since SQ-1199, still ENCODING on the tile worker — so the grid fills
        // in without needing a keypress.
        let gallery_busy = matches!(view, PickerView::Gallery)
            && (!requested.is_empty() || tile_encoder.pending());
        let cover_busy = panel_busy || gallery_busy;
        let search_busy = search_modal.as_ref().is_some_and(|m| m.busy()) || search_worker.busy();
        // The modal's own lists ease exactly as `list` does (SQ-0598), so they
        // need the same tick — without it a scroll would freeze mid-tween until
        // the next keypress.
        let search_scrolling =
            search_modal.as_ref().is_some_and(|m| m.has_active_animation());
        // SQ-1213: while the gallery's scroll-settle window is open, keep
        // ticking so the redraw that turns a suppressed sixel tile back into
        // its real payload fires on its own, without waiting for another key —
        // mirroring `has_active_animation()` pulling in `transcript_scroll_in_motion`
        // for the transcript's own debounce (SQ-1198).
        if (list.has_active_animation() || slide.active() || cover_busy || fetcher.busy() || hint_dl.busy() || url_dl.busy() || search_busy || search_scrolling || gallery_scroll_in_motion(gallery_scroll_motion_at))
            && !crossterm::event::poll(Duration::from_millis(16)).unwrap_or(false)
        {
            list.finalize_if_done();
            if let Some(m) = &mut search_modal {
                m.finalize_if_done();
            }
            continue;
        }

        // Wait for the next event via a bounded poll instead of a plain blocking
        // read(): crossterm swallows the EINTR a signal delivers (and signal-hook
        // uses SA_RESTART), so an idle blocking read() would never observe the
        // termination flag. Re-check it each ~100ms tick (no redraw) so a
        // kill/SIGHUP restores the terminal promptly instead of hanging.
        loop {
            exit_if_terminated();
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => break,  // an event is ready → read it below
                Ok(false) => {}     // timeout → re-check the flag, keep waiting
                Err(_) => break,    // let read() below surface the error
            }
        }

        // A closed controlling terminal makes the poll above report "ready" (HUP)
        // on the dead fd, breaking the loop — but `read()` would then block forever
        // on that fd, never re-checking the flag. The signal handler has already set
        // it by now, so catch it here before the blocking read. (SQ-0502)
        exit_if_terminated();

        // This iteration's browser gesture, applied AFTER the event match so
        // that a key and a story-menu item reach one dispatch (SQ-1227). A key
        // is carried rather than resolved here because resolving it is the
        // dispatch's own first act, and the guard test below requires that act
        // to sit inside the marked region with the rest of it.
        let mut pending_key = None;
        let mut pending_command: Option<&'static str> = None;

        match read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                use crossterm::event::KeyCode::*;
                let shift = k.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
                // The launch-options dialog (SQ-0789) captures all keys while
                // open; `LaunchOptionsState::on_key` owns the model (the
                // settings screen's: ↑/↓ rows, Space acts on the row under the
                // cursor, Tab/Shift-Tab buttons, Enter activates, Esc cancels).
                if let Some(lo) = launch_opts.as_mut() {
                    match lo.on_key(k) {
                        app::launch_options::LaunchOptionsAction::Play => {
                            let lo = launch_opts.take().expect("open");
                            // The checkbox is the whole "try before you commit"
                            // idea: the options apply to this launch either way,
                            // and only a ticked box writes them down.
                            if lo.persist {
                                // Keyed on the story the dialog was opened
                                // for, which on a compilation is not the one
                                // the image's path resolves to (SQ-0859).
                                let game_dir = app::storage::game_dir(
                                    data_base,
                                    &app::storage::story_key_at_from(
                                        &lo.story_path,
                                        lo.disk_entry.as_deref(),
                                    ),
                                );
                                if let Err(e) = lo.persist_to(&game_dir) {
                                    // Said after the alternate screen is torn
                                    // down (below), so it survives in scrollback
                                    // rather than being wiped by the game the
                                    // very next instant — the same reasoning
                                    // SQ-0734 applied to its own warning.
                                    persist_error =
                                        Some(format!("could not save launch options: {e}"));
                                }
                            }
                            break Some(PickedStory {
                                path: lo.story_path.clone(),
                                disk_entry: lo.disk_entry.clone(),
                                overrides: lo.overrides(),
                            });
                        }
                        app::launch_options::LaunchOptionsAction::Cancel => launch_opts = None,
                        app::launch_options::LaunchOptionsAction::None => {}
                    }
                // The key reference (SQ-1227) captures all keys while open: Esc,
                // `?` again, `q` or Enter close it, everything else is swallowed
                // rather than acting on the list behind it.
                } else if keys_dialog {
                    if matches!(k.code, Esc | Enter | Char('q') | Char('?')) {
                        keys_dialog = false;
                    }
                // The per-story menu (SQ-1227) captures all keys while open. It
                // owns the model — ↑/↓ wrap, Enter activates, Esc closes, and an
                // item's own hotkey activates it directly — and hands back the
                // command-string to run, which goes through the ONE dispatch
                // below exactly as if the key had been pressed on the list.
                } else if let Some(menu) = story_menu.as_mut() {
                    match menu.on_key(k, &keymap) {
                        app::story_menu::MenuOutcome::Activate(cmd) => {
                            story_menu = None;
                            pending_command = Some(cmd);
                        }
                        app::story_menu::MenuOutcome::Close => story_menu = None,
                        app::story_menu::MenuOutcome::None => {}
                    }
                // The IFDB search modal (SQ-0413) captures all keys while open;
                // its state machine decides what each does (Esc backs out a
                // level, Enter activates, ↑/↓/j/k navigate).
                } else if search_modal.is_some() {
                    let action = search_modal.as_mut().unwrap().on_key(k.code, anim);
                    dispatch_search_action(action, &search_worker, &dir, &mut search_modal);
                // The resource-preview modal (SQ-0347) captures all keys while
                // open: `+`/`=`/`-`/`0` step the zoom (SQ-0486, intercepted
                // ahead of dismissal); any of Esc/Enter/q/Space dismisses it
                // (and stops a sound).
                } else if preview.is_some() {
                    match k.code {
                        Char('+') | Char('=') => {
                            preview.as_mut().unwrap().zoom = preview.as_ref().unwrap().zoom.step_in();
                        }
                        Char('-') => {
                            preview.as_mut().unwrap().zoom = preview.as_ref().unwrap().zoom.step_out();
                        }
                        Char('0') => {
                            preview.as_mut().unwrap().zoom = PreviewZoom::Fit;
                        }
                        Esc | Enter | Char('q') | Char(' ') => {
                            // Free the modal's upload before dropping the struct
                            // that names it (SQ-1190) — `preview = None` alone
                            // would only forget the id, not free it.
                            if let Some(old) = preview.take() {
                                cover.queue_external_delete(old.proto.and_then(|t| t.4));
                            }
                            if let Some(a) = audio.as_mut() {
                                a.stop_all();
                            }
                        }
                        _ => {}
                    }
                } else if let Some(field) = find_field.as_mut() {
                    // Type-to-find: letters edit the query and the list is the
                    // whole library's matches. The vertical keys still move the
                    // selection, so a match is picked without leaving the field;
                    // Enter opens it; Esc puts the folder view back.
                    let mut refilter = false;
                    match k.code {
                        Esc => {
                            find_field = None;
                            panel_scroll = 0;
                            if gallery_all_folders(view, false, index_rx.is_some()) {
                                show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                            } else {
                                let here = dir.clone();
                                enter_folder(&source, &mut dir, &root, &here, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index, viewport, anim);
                            }
                        }
                        Enter => {
                            if let Some(entry) = stories.get(list.selected) {
                                break Some(PickedStory::row(entry));
                            }
                        }
                        Up | Down | PageUp | PageDown | Home | End => {
                            panel_scroll = 0;
                            app::list_scroll::nav_key(&mut list, k.code, stories.len(), viewport, anim);
                        }
                        Backspace => {
                            field.backspace();
                            refilter = true;
                        }
                        Delete => {
                            field.delete();
                            refilter = true;
                        }
                        Left => field.left(),
                        Right => field.right(),
                        // A control chord (Ctrl+F itself, most likely) is not a
                        // character for the query.
                        Char(c) if !k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                            field.insert(c);
                            refilter = true;
                        }
                        _ => {}
                    }
                    if refilter {
                        panel_scroll = 0;
                        let query = find_field.as_ref().map(|f| f.as_str().to_string()).unwrap_or_default();
                        apply_find(&index, &root, &query, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                    }
                } else if let Some(field) = manual_ifdb.as_mut() {
                    match k.code {
                        Esc => {
                            manual_ifdb = None;
                            progress_line = None;
                        }
                        Enter => {
                            let input = field.take();
                            manual_ifdb = None;
                            if let Some(entry) = stories.get(list.selected) {
                                match app::ifdb::extract_tuid(&input) {
                                    Some(tuid) => {
                                        fetch_is_single = true;
                                        sweep_fetched = 0;
                                        sweep_skipped = 0;
                                        sweep_not_found = 0;
                                        sweep_failed = 0;
                                        progress_line = Some(format!("Fetching {} from IFDB…", entry.title));
                                        fetcher.request(app::fetch_worker::FetchOrder {
                                            stories: vec![app::fetch_worker::FetchTarget::row(entry)],
                                            forced: true,
                                            id_override: Some(tuid),
                                        });
                                    }
                                    None => {
                                        progress_line = Some("Not an IFDB URL or id".to_string());
                                    }
                                }
                            }
                        }
                        Backspace => field.backspace(),
                        Delete => field.delete(),
                        Left => field.left(),
                        Right => field.right(),
                        Home => field.home(),
                        End => field.end(),
                        Char(c) => field.insert(c),
                        _ => {}
                    }
                } else if let Some(field) = url_prompt.as_mut() {
                    // SQ-1086: the open-a-URL prompt. Same shape as the manual
                    // IFDB field above — Enter submits, Esc cancels, everything
                    // else edits — because a second editing idiom in one footer
                    // row would be a second thing to learn for no reason.
                    match k.code {
                        Esc => {
                            url_prompt = None;
                            progress_line = None;
                        }
                        Enter => {
                            let input = field.take();
                            url_prompt = None;
                            let typed = input.trim().to_string();
                            if typed.is_empty() {
                                progress_line = None;
                            } else if !app::story_url::is_story_url(&typed) {
                                // Say which of the two mistakes it is: a URL
                                // scheme lanthorn will not fetch, or something
                                // that is not an address at all.
                                progress_line = Some(
                                    app::story_url::declined_scheme(&typed).unwrap_or_else(|| {
                                        "Not a URL — paste an http:// or https:// address".to_string()
                                    }),
                                );
                            } else if url_dl.busy() {
                                progress_line = Some("A download is already running".to_string());
                            } else {
                                progress_line = Some(format!("Downloading {typed}…"));
                                url_dl.start(typed, dir.clone());
                            }
                        }
                        Backspace => field.backspace(),
                        Delete => field.delete(),
                        Left => field.left(),
                        Right => field.right(),
                        Home => field.home(),
                        End => field.end(),
                        Char(c) => field.insert(c),
                        _ => {}
                    }
                } else if slide.open && shift && matches!(k.code, Up | Down | PageUp | PageDown) {
                    // Shift + a scroll key drives the open info panel. Only these
                    // keys are intercepted — any other shift combo (e.g. `H` to
                    // download hints) must still fall through to normal handling.
                    let page = (last_panel_area.height.saturating_sub(2)).max(1) as usize;
                    match k.code {
                        Up => panel_scroll = panel_scroll.saturating_sub(1),
                        Down => panel_scroll = (panel_scroll + 1).min(panel_max),
                        PageUp => panel_scroll = panel_scroll.saturating_sub(page),
                        PageDown => panel_scroll = (panel_scroll + page).min(panel_max),
                        _ => {}
                    }
                } else {
                    // A finished fetch leaves its summary on the footer row; the
                    // next keypress (once nothing is in flight) clears it so the
                    // normal hints return — otherwise the summary would sit there
                    // for the rest of the session, hiding the key legend.
                    if !fetcher.busy() {
                        progress_line = None;
                    }
                    // Nothing above claimed the key, so it is the browser's:
                    // carried to the one dispatch below, which is the only place
                    // a key becomes an action (SQ-1227).
                    pending_key = Some(k);
                }
            }
            Ok(Event::Mouse(m)) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                // Pixel mouse reporting (SQ-0563) is terminal-wide state: once a
                // game has switched it on, coming back here (Change Story) would
                // otherwise hand this loop pixel coordinates. The launcher has no
                // use for sub-cell precision, so take the cells and drop the rest.
                let (m, _) = app::pixel_mouse::normalise(m);
                if let MouseEventKind::Down(MouseButton::Right) = m.kind {
                    // SQ-1227: a SINGLE right-click on a row opens that story's
                    // menu — selecting the row first if it was not the
                    // highlighted one, so the menu and the selection can never
                    // disagree about which story is being talked about.
                    //
                    // This replaces SQ-0789's double-right-click shortcut to the
                    // launch-options dialog. The intent survives — a story can
                    // still be started some way other than the default one, and
                    // still from the mouse — but it is now an item you can SEE
                    // rather than a gesture nothing on screen mentioned.
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    if launch_opts.is_none()
                        && search_modal.is_none()
                        && preview.is_none()
                        && !keys_dialog
                    {
                        let hit = row_rects
                            .iter()
                            .find(|(_, r)| r.contains(pt))
                            .map(|(i, _)| (*i, stories.get(*i).is_some_and(|e| e.is_folder())));
                        let (select, open) = right_click_action(hit);
                        if let Some(idx) = select.filter(|i| *i != list.selected) {
                            panel_scroll = 0;
                            list.select(idx, viewport, anim);
                        }
                        story_menu = open.map(app::story_menu::StoryMenu::new);
                    }
                } else if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    if keys_dialog {
                        // The key reference (SQ-1227): ✕, Done, or a click
                        // outside closes it; a click inside is swallowed.
                        let on_close = keys_close_rect.is_some_and(|r| r.contains(pt));
                        let on_button = keys_button_rects.iter().any(|(_, r)| r.contains(pt));
                        if on_close || on_button || !keys_area.contains(pt) {
                            keys_dialog = false;
                        }
                    } else if story_menu.is_some() {
                        // The per-story menu (SQ-1227): a click on an item runs
                        // it, anywhere else dismisses. The click never falls
                        // through to the row underneath — a menu you dismiss by
                        // clicking past it must not also move the selection.
                        match menu_rects.iter().find(|(_, r)| r.contains(pt)) {
                            Some((i, _)) => {
                                story_menu = None;
                                pending_command =
                                    app::story_menu::STORY_MENU.get(*i).map(|it| it.command);
                            }
                            // Its own border is not "outside": a click that
                            // lands on the frame is a miss, not a dismissal.
                            None if menu_area.contains(pt) => {}
                            None => story_menu = None,
                        }
                    } else if let Some(lo) = launch_opts.as_mut() {
                        // Topmost modal: ✕ / Cancel / outside dismiss, a row moves
                        // the cursor and acts on it (one click = point and choose),
                        // Play launches with whatever is selected.
                        let on_close = launch_close_rect.is_some_and(|r| r.contains(pt));
                        let button = launch_button_rects
                            .iter()
                            .find(|(_, r)| r.contains(pt))
                            .map(|(id, _)| *id);
                        let row = launch_row_rects.iter().find(|(_, r)| r.contains(pt)).map(|(i, _)| *i);
                        if let Some(i) = row {
                            lo.set_cursor_index(i);
                            lo.on_key(crossterm::event::KeyEvent::new(
                                crossterm::event::KeyCode::Char(' '),
                                crossterm::event::KeyModifiers::NONE,
                            ));
                        } else if button == Some(app::render::dialog::ButtonId::PlayAgain) {
                            let lo = launch_opts.take().expect("open");
                            if lo.persist {
                                // Keyed on the story the dialog was opened
                                // for, which on a compilation is not the one
                                // the image's path resolves to (SQ-0859).
                                let game_dir = app::storage::game_dir(
                                    data_base,
                                    &app::storage::story_key_at_from(
                                        &lo.story_path,
                                        lo.disk_entry.as_deref(),
                                    ),
                                );
                                if let Err(e) = lo.persist_to(&game_dir) {
                                    // Said after the alternate screen is torn
                                    // down (below), so it survives in scrollback
                                    // rather than being wiped by the game the
                                    // very next instant — the same reasoning
                                    // SQ-0734 applied to its own warning.
                                    persist_error =
                                        Some(format!("could not save launch options: {e}"));
                                }
                            }
                            break Some(PickedStory {
                                path: lo.story_path.clone(),
                                disk_entry: lo.disk_entry.clone(),
                                overrides: lo.overrides(),
                            });
                        } else if on_close
                            || button == Some(app::render::dialog::ButtonId::Cancel)
                            || !launch_area.contains(pt)
                        {
                            launch_opts = None;
                        }
                    } else if search_modal.is_some() {
                        // IFDB search modal (SQ-0413): the ✕ or a click outside the
                        // dialog closes it; a click inside is swallowed (its lists
                        // are keyboard-driven).
                        let on_close = search_close_rect.is_some_and(|r| r.contains(pt));
                        if on_close || !search_area.contains(pt) {
                            search_modal = None;
                        }
                    } else if preview.is_some() {
                        // Modal open (SQ-0347): the ✕, the Close button, or a click
                        // outside the dialog all dismiss it (and stop a sound); a
                        // click inside is swallowed.
                        let on_close = preview_close_rect.is_some_and(|r| r.contains(pt));
                        let on_button = preview_button_rects.iter().any(|(_, r)| r.contains(pt));
                        let outside = !preview_area.contains(pt);
                        if on_close || on_button || outside {
                            // See the keyboard-dismiss arm above (SQ-1190).
                            if let Some(old) = preview.take() {
                                cover.queue_external_delete(old.proto.and_then(|t| t.4));
                            }
                            if let Some(a) = audio.as_mut() {
                                a.stop_all();
                            }
                        }
                    } else if let Some((_, rref)) = panel_resource_rects.iter().find(|(r, _)| r.contains(pt)) {
                        // Click on a previewable Pict/Snd resource row (SQ-0347):
                        // open its modal (image renders / sound plays).
                        preview = Some(open_resource_preview(rref, &mut audio, cfg.volume));
                    } else if let Some((_, url)) = panel_link_rects.iter().find(|(r, _)| r.contains(pt)) {
                        // Click on an info-panel OSC 8 link (SQ-0367): the terminal
                        // can't act on it while we hold mouse capture, so open it.
                        open_url(url);
                    } else if let Some((idx, _)) = row_rects.iter().find(|(_, r)| r.contains(pt)) {
                        let idx = *idx;
                        let now = Instant::now();
                        // Second click on the already-selected row within the
                        // window → launch; otherwise just select it (SQ-0366).
                        let double = last_click
                            .is_some_and(|(li, lt)| li == idx && now.duration_since(lt) < DOUBLE_CLICK);
                        if double && stories[idx].is_folder() {
                            // A double-click on a folder enters it, like Enter.
                            let target = stories[idx].path.clone();
                            panel_scroll = 0;
                            enter_folder(&source, &mut dir, &root, &target, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index, viewport, anim);
                            last_click = None;
                        } else if double {
                            break Some(PickedStory::row(&stories[idx]));
                        } else {
                            panel_scroll = 0;
                            list.select(idx, viewport, anim);
                            if slide.open {
                                ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
                            }
                            last_click = Some((idx, now));
                        }
                    } else if let Some((key, _)) = header_rects.iter().find(|(_, r)| r.contains(pt)) {
                        // Click the active header → reverse; click another → sort
                        // by it, ascending.
                        if sort.key == *key {
                            sort.desc = !sort.desc;
                        } else {
                            sort.key = *key;
                            sort.desc = false;
                        }
                        list.select(
                            resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                            viewport,
                            anim,
                        );
                    }
                } else if let Some(d) = app::input::wheel_delta(m.kind, cfg.mouse_wheel_invert) {
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    match wheel_target(
                        launch_opts.is_some(),
                        keys_dialog,
                        story_menu.is_some(),
                        search_modal.is_some(),
                        preview.is_some(),
                        slide.open && last_panel_area.contains(pt),
                    ) {
                        // A modal with nothing to scroll still eats the notch.
                        WheelTarget::Swallowed => {}
                        // The IFDB search modal owns the wheel while it is open,
                        // the same precedence its clicks already take — it
                        // scrolls its own results/files list, and never the
                        // story list behind it (SQ-0831).
                        WheelTarget::Search => {
                            if let Some(sm) = search_modal.as_mut() {
                                sm.on_wheel(d, anim);
                            }
                        }
                        // Over the preview modal, the wheel zooms instead of
                        // scrolling the list behind it (SQ-0486; a no-op prior to
                        // that, per SQ-0347): up zooms in, down zooms out.
                        WheelTarget::PreviewZoom => {
                            if let Some(pv) = preview.as_mut() {
                                pv.zoom = if d < 0 { pv.zoom.step_in() } else { pv.zoom.step_out() };
                            }
                        }
                        WheelTarget::InfoPanel => {
                            if d < 0 {
                                panel_scroll = panel_scroll.saturating_sub((-d) as usize);
                            } else {
                                panel_scroll = (panel_scroll + d as usize).min(panel_max);
                            }
                        }
                        // Record the notch's direction; the coalesced step is
                        // applied at the loop top once this notch's event burst
                        // drains, so one notch scrolls the list exactly one row.
                        WheelTarget::StoryList => pending_wheel = Some(d),
                    }
                }
            }
            Ok(Event::Resize(_, _)) => {
                let _ = terminal.clear();
                // SQ-0988: the cell may have changed shape, not just the grid.
                // Every built cover raster was aspect-fitted against the old one.
                if cover_picker.as_mut().is_some_and(refresh_cell_size) {
                    cover.invalidate_cell_geometry();
                }
            }
            Ok(_) => {}
            Err(_) => break None,
        }

        // ── BROWSER KEY DISPATCH (registry-driven, SQ-0796) ─────────────────
        // Everything below is keyed on a `BrowserAction`, and the only thing
        // that produces one is a `slash::COMMANDS` entry in `Context::Browser`
        // — reached either by a key bound to it or by the story menu's item for
        // it (SQ-1227), which is why this sits outside the event match: one
        // dispatch, whichever gesture asked. Nothing in this region may look at
        // the keystroke again — that is what makes a new gesture impossible to
        // add without a registry entry, and it is pinned by
        // `browser_dispatch_never_reads_the_key_event` below.
        let action = pending_command
            .and_then(app::browser::action_for_command)
            .or_else(|| pending_key.and_then(|ev| app::browser::action_for_key(&keymap, ev)));
        // Gallery navigation moves a 2D cursor over the same shared selection;
        // the list moves it linearly. `gm` computes the clamped grid target for
        // a (dx, dy) step.
        let gm = |sel: usize, dx: isize, dy: isize| {
            app::cover_gallery::move_index(sel, gallery_cols, stories.len(), dx, dy)
        };
        let gallery = matches!(view, PickerView::Gallery);
        match action {
            // Movement. In the grid this is a 2D cursor; in the list
            // it is the shared `list_scroll::nav_key` (SQ-0682) — the
            // same mechanism the IFDB search modal's lists and the
            // command band's columns navigate with — and a horizontal
            // step has no meaning there, so it does nothing at all.
            Some(app::browser::BrowserAction::MoveSelection { dx, dy }) => {
                if gallery {
                    panel_scroll = 0;
                    list.select(gm(list.selected, dx, dy), viewport, anim);
                    gallery_scroll_motion_at = Some(Instant::now());
                } else if let Some(nav) = action.and_then(app::browser::list_nav_code) {
                    panel_scroll = 0;
                    app::list_scroll::nav_key(&mut list, nav, stories.len(), viewport, anim);
                }
            }
            Some(app::browser::BrowserAction::PageSelection(n)) => {
                panel_scroll = 0;
                if gallery {
                    list.select(gm(list.selected, 0, n * gallery_vis as isize), viewport, anim);
                    gallery_scroll_motion_at = Some(Instant::now());
                } else if let Some(nav) = action.and_then(app::browser::list_nav_code) {
                    app::list_scroll::nav_key(&mut list, nav, stories.len(), viewport, anim);
                }
            }
            // List view only (SQ-1228): the cover gallery has no
            // half-row concept, so Ctrl-U/Ctrl-D do nothing there.
            Some(app::browser::BrowserAction::HalfPageSelection(n)) => {
                panel_scroll = 0;
                if !gallery {
                    let dir = if n < 0 { -1 } else { 1 };
                    list.half_page(dir, viewport, anim);
                }
            }
            Some(app::browser::BrowserAction::SelectEdge(edge)) => {
                panel_scroll = 0;
                if gallery {
                    match edge {
                        app::browser::Edge::First => list.select(0, viewport, anim),
                        app::browser::Edge::Last => {
                            list.select(stories.len().saturating_sub(1), viewport, anim)
                        }
                    }
                    gallery_scroll_motion_at = Some(Instant::now());
                } else if let Some(nav) = action.and_then(app::browser::list_nav_code) {
                    app::list_scroll::nav_key(&mut list, nav, stories.len(), viewport, anim);
                }
            }
            // `.get`, not indexing (SQ-0659): playing an empty list
            // (all stories vanished externally) is a no-op, not a
            // panic.
            Some(app::browser::BrowserAction::PlayStory) => match stories.get(list.selected) {
                // A folder is entered, not played.
                Some(entry) if entry.is_folder() => {
                    let target = entry.path.clone();
                    panel_scroll = 0;
                    enter_folder(&source, &mut dir, &root, &target, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index, viewport, anim);
                }
                Some(entry) => break Some(PickedStory::row(entry)),
                None => {}
            },
            // `o`, Shift-Enter and the story menu's own row are one
            // command reaching one constructor (SQ-0789): the dialog
            // has a single seeding site, and a single binding target
            // as well.
            Some(app::browser::BrowserAction::OpenLaunchOptions) => {
                if let Some(entry) = stories.get(list.selected).filter(|e| !e.is_folder()) {
                    launch_opts = Some(open_launch_options(entry, cfg, data_base));
                }
            }
            // The per-story menu (SQ-1227). A folder has none of these
            // actions — it is entered, not played — so the gesture is
            // inert on one, exactly as launch options already are.
            Some(app::browser::BrowserAction::OpenStoryMenu) => {
                if stories.get(list.selected).is_some_and(|e| !e.is_folder()) {
                    story_menu = Some(app::story_menu::StoryMenu::new(list.selected));
                }
            }
            Some(app::browser::BrowserAction::ShowBrowserKeys) => {
                keys_dialog = true;
                progress_line = None;
            }
            // Open the find field over the in-memory index. An empty
            // query lists the whole library, which is itself the
            // answer to "where did that game go" in a tree.
            Some(app::browser::BrowserAction::FindStory) => {
                if index_rx.is_some() && find_field.is_none() {
                    find_field = Some(app::text_field::TextField::new(""));
                    progress_line = None;
                    panel_scroll = 0;
                    apply_find(&index, &root, "", &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                }
            }
            // Up one folder; inert at the root.
            Some(app::browser::BrowserAction::ParentFolder) => {
                if dir != root {
                    if let Some(parent) = dir.parent().map(|p| p.to_path_buf()) {
                        panel_scroll = 0;
                        if gallery_all_folders(view, find_field.is_some(), index_rx.is_some()) {
                            dir = parent;
                            show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                        } else {
                            enter_folder(&source, &mut dir, &root, &parent, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index, viewport, anim);
                        }
                    }
                }
            }
            Some(app::browser::BrowserAction::QuitBrowser) => break None,
            // Cancels a running sweep first; only quits when nothing
            // is in flight.
            Some(app::browser::BrowserAction::CancelBrowser) => {
                if fetcher.busy() {
                    fetcher.cancel();
                } else {
                    break None;
                }
            }
            Some(app::browser::BrowserAction::ToggleInfoPanel) => {
                let target = !slide.open;
                if !target || can_open_panel(last_area.width) {
                    let instant = !cfg.animation.enabled || cfg.animation.scroll_ms == 0;
                    slide.toggle_to(target, instant);
                    slide.arm(&cfg.animation);
                    if target {
                        panel_scroll = 0;
                        ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
                    }
                }
            }
            // Toggle the cover-gallery grid (SQ-0374). Selection
            // carries over; reset the grid scroll so the selected
            // cover is framed on entry (the next draw scrolls to it).
            Some(app::browser::BrowserAction::ToggleGallery) => {
                view = match view {
                    PickerView::List => PickerView::Gallery,
                    PickerView::Gallery => PickerView::List,
                };
                gallery_first_row = 0;
                // The grid shows the folder and everything under
                // it; the list shows the folder's own rows. Swap
                // the list to match, unless a find is showing
                // matches in both.
                if find_field.is_none() && index_rx.is_some() {
                    panel_scroll = 0;
                    if matches!(view, PickerView::Gallery) {
                        show_gallery_scope(&index, &root, &dir, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index);
                    } else {
                        let keep = stories.get(list.selected).map(|e| e.path.clone());
                        let here = dir.clone();
                        enter_folder(&source, &mut dir, &root, &here, &mut stories, &mut row_badges, &mut aux_cache, &mut list, data_base, &hint_index, viewport, anim);
                        if let Some(idx) = keep.and_then(|p| stories.iter().position(|e| e.path == p)) {
                            list.select(idx, viewport, anim);
                        }
                    }
                }
            }
            // Refetch only the selected story, ignoring its cache.
            // Ignored while a sweep is already running, so a second
            // press can't garble the in-flight progress line.
            Some(app::browser::BrowserAction::FetchStory) => {
                if let Some(entry) = stories.get(list.selected).filter(|e| !e.is_folder() && !fetcher.busy()) {
                    fetch_is_single = true;
                    sweep_fetched = 0;
                    sweep_skipped = 0;
                    sweep_not_found = 0;
                    sweep_failed = 0;
                    progress_line = Some(format!("Fetching {}…", entry.title));
                    fetcher.request(app::fetch_worker::FetchOrder {
                        stories: vec![app::fetch_worker::FetchTarget::row(entry)],
                        forced: true,
                        id_override: None,
                    });
                }
            }
            // Sweep the whole library; the worker itself skips any
            // story already at the current FETCH_VERSION. Ignored
            // while a sweep is already running (see fetch-story).
            Some(app::browser::BrowserAction::RefreshLibrary) => {
                // A busy-worker check is an `if` inside the arm, never
                // a match guard: a guarded arm does not count towards
                // exhaustiveness, and it is exhaustiveness here that
                // makes a new `BrowserAction` a compile error rather
                // than a gesture that quietly does nothing.
                if !fetcher.busy() {
                    // Folder rows are not stories; the sweep skips them.
                    let order: Vec<app::fetch_worker::FetchTarget> = stories
                        .iter()
                        .filter(|e| !e.is_folder())
                        .map(app::fetch_worker::FetchTarget::row)
                        .collect();
                    let total = order.len();
                    fetch_is_single = false;
                    sweep_fetched = 0;
                    sweep_skipped = 0;
                    sweep_not_found = 0;
                    sweep_failed = 0;
                    progress_line = Some(format!("Fetching 0/{total}"));
                    fetcher.request(app::fetch_worker::FetchOrder { stories: order, forced: false, id_override: None });
                }
            }
            // Point the selected story at an IFDB page by hand (for a
            // story whose IFID IFDB doesn't index). Opens the
            // manual-entry field; ignored mid-sweep (SQ-0371).
            Some(app::browser::BrowserAction::SetIfdbUrl) => {
                if !fetcher.busy() && stories.get(list.selected).is_some_and(|e| !e.is_folder()) {
                    manual_ifdb = Some(app::text_field::TextField::new(""));
                    progress_line = None;
                }
            }
            // Open the IFDB search modal (SQ-0413) — search by
            // title/author, browse results, and download a story file
            // into this directory. Opens on a "Popular on IFDB" seed
            // list (SQ-0473), fetched non-blocking through the same
            // worker.
            // Open a story straight from a URL (SQ-1086). It lands
            // in `dir`, which IS the library, so the download is kept
            // by construction — the command line's keep-it prompt has
            // no counterpart here.
            Some(app::browser::BrowserAction::OpenUrl) => {
                if !url_dl.busy() {
                    url_prompt = Some(app::text_field::TextField::new(""));
                    progress_line = None;
                }
            }
            Some(app::browser::BrowserAction::SearchIfdb) => {
                let mut sm = app::ifdb_search_modal::SearchModal::new();
                // So the chooser can mark files this directory
                // already holds (SQ-0597) — the same `dir` every
                // download lands in, below.
                sm.set_download_dir(&dir);
                let seed_action = sm.open();
                search_modal = Some(sm);
                dispatch_search_action(seed_action, &search_worker, &dir, &mut search_modal);
                progress_line = None;
            }
            // Download a matching InvisiClues hint file for the
            // selected story (SQ-0445) when it has none locally — SLAG
            // (IF Archive) preferred, else the Internet Archive izm set.
            // Saved beside the story; ignored while one is downloading.
            Some(app::browser::BrowserAction::DownloadHints) => {
                if let Some(entry) = stories.get(list.selected).filter(|e| !e.is_folder() && !hint_dl.busy()) {
                    if entry.hint_sidecar.is_some() {
                        progress_line = Some(format!("{} already has a hint file", entry.title));
                    } else {
                        let stem =
                            entry.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        match app::hints::hint_download_for(
                            &entry.meta.ifid,
                            stem,
                            &entry.title,
                        ) {
                            Some(dl) => {
                                let dest = entry.path.with_file_name(&dl.filename);
                                progress_line =
                                    Some(format!("Downloading hints for {}…", entry.title));
                                hint_dl.start(
                                    dl.url,
                                    dest,
                                    entry.path.clone(),
                                    entry.meta.disk_entry.clone(),
                                    entry.title.clone(),
                                );
                            }
                            None => {
                                progress_line =
                                    Some(format!("No InvisiClues found for {}", entry.title));
                            }
                        }
                    }
                }
            }
            // Cycle the sort column, keeping direction; or toggle the
            // direction, keeping the column. Both preserve the
            // selection by path, never by index.
            Some(app::browser::BrowserAction::SortLibrary) => {
                sort.key = match sort.key {
                    app::picker::SortKey::Title => app::picker::SortKey::Author,
                    app::picker::SortKey::Author => app::picker::SortKey::Year,
                    app::picker::SortKey::Year => app::picker::SortKey::Rating,
                    app::picker::SortKey::Rating => app::picker::SortKey::Type,
                    app::picker::SortKey::Type => app::picker::SortKey::Title,
                };
                list.select(
                    resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                    viewport,
                    anim,
                );
            }
            Some(app::browser::BrowserAction::ReverseSort) => {
                sort.desc = !sort.desc;
                list.select(
                    resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                    viewport,
                    anim,
                );
            }
            // An unbound key. The ONLY catch-all in this match, so the
            // compiler still requires an arm per action above.
            None => {}
        }
        // ── END BROWSER KEY DISPATCH ────────────────────────────────

        panel_scroll = panel_scroll.min(panel_max);
        list.finalize_if_done();
    };

    restore_terminal();
    if let Some(msg) = persist_error {
        eprintln!("lanthorn: {msg}");
    }
    chosen
}

/// Per-row hit-rects (row index, rect) for mouse selection.
type RowHitRects = Vec<(usize, Rect)>;
/// Column-header hit-rects (sort key, rect) for click-to-sort.
type HeaderHitRects = Vec<(app::picker::SortKey, Rect)>;

/// Draw the story-picker screen. Returns the per-row hit-rects (index, rect)
/// for mouse selection, the row count, and the column-header hit-rects
/// (Task 9 hit-tests these for click-to-sort).
///
/// `km` is the resolved keymap, read only to name the keys in the footer hints
/// (SQ-0796) — the drawing itself never consults it.
#[allow(clippy::too_many_arguments)]
fn draw_story_picker(
    stories: &[app::picker::StoryEntry],
    list: &app::list_scroll::ListScroll,
    badges: &[app::picker::RowBadges],
    glyphs: &app::picker::BadgeGlyphs,
    heading: &PickerHeading,
    cs: &app::colors::ColorScheme,
    km: &app::keymap::KeyMap,
    sort: app::picker::Sort,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) -> (RowHitRects, usize, HeaderHitRects) {
    use app::picker::SortKey;
    use ratatui::style::{Color, Style};
    let selected = list.selected;
    let mut row_rects: Vec<(usize, Rect)> = Vec::new();
    let mut header_rects: Vec<(SortKey, Rect)> = Vec::new();

    let dialog = cs.theme.get("dialog.background").style;
    let dialog_title = cs.theme.get("dialog.title").style;
    let story_header = cs.theme.get("story_header").style;
    let story_header_active = cs.theme.get("story_header_active").style;
    let dialog_button_active = cs.theme.get("dialog.button:active").style;
    let story_author = cs.theme.get("story_author").style;
    let story_no_metadata = cs.theme.get("story_no_metadata").style;
    let story_year = cs.theme.get("story_year").style;
    let story_rating = cs.theme.get("story_rating").style;
    let story_badge = cs.theme.get("story_badge").style;
    let story_folder = cs.theme.get("story_folder").style;
    let scrollbar = app::render::scroll::ScrollbarLook::from_theme(&cs.theme);

    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(dialog);
            }
        }
    }

    // Header.
    let header = heading.line(stories, "g: covers");
    draw_str_clipped(buf, area.x, area.y, &header, dialog_title, area);

    // List region (title bar + column-header row at top, footer at bottom).
    let list_top = area.y + 2;
    let list_bottom = area.bottom().saturating_sub(1);
    if list_bottom <= list_top {
        return (row_rects, 0, header_rects);
    }
    let rows = (list_bottom - list_top) as usize;
    let total = stories.len();

    // Reserve a 1-col gutter for the scrollbar when the list overflows.
    let scrollbar_visible =
        app::render::scroll::needs_scrollbar(total, rows) && area.width >= 2;
    let row_w = if scrollbar_visible { area.width.saturating_sub(1) } else { area.width };
    let first = list.display_offset();

    // Badge cluster width depends only on the configured glyphs, not the
    // entry, so it's computed once and reused both to size the text columns
    // and to place each row's cluster. Both the interpreter/format AND the
    // blorb indicator moved into the TYPE column (SQ-0369: "Z5 (blorb)"), so
    // the cluster is now just [save][hint].
    let save_w = glyphs.save.chars().count() as u16;
    // Reserve the wider of the present/available hint glyphs so the cluster width
    // is stable regardless of which one a row shows.
    let hint_w = glyphs.hint.chars().count().max(glyphs.hint_available.chars().count()) as u16;
    let cluster_w = save_w + hint_w;
    // The TYPE column rides in the same right-hand zone as the badges, one gap
    // to their left; both are shown together or dropped together.
    let right_zone = INTERP_COL_W + COL_GAP + cluster_w;
    let badges_shown = right_zone + 2 < row_w;
    let badge_reserved = if badges_shown { right_zone + 1 } else { 0 };
    let text_w = row_w.saturating_sub(badge_reserved);
    // Widest author name across the WHOLE list (not just the visible page), so
    // the author column doesn't jump width as the user scrolls.
    let want_author_w = stories
        .iter()
        .filter_map(|e| e.meta.author.as_deref())
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0) as u16;
    let cols = compute_columns(text_w, want_author_w);

    let title_x = area.left() + ROW_MARKER_W;
    let author_x = title_x + cols.title_w + COL_GAP;
    let year_x = author_x + cols.author_w + COL_GAP;
    let rating_x = year_x + cols.year_w + COL_GAP;

    // Column-header row: dimmed, except the active sort column, which shows
    // its direction arrow.
    let header_y = area.y + 1;
    let (title_label, title_active) = header_label("TITLE", SortKey::Title, sort);
    let title_hstyle = if title_active { story_header_active } else { story_header };
    draw_str_clipped(buf, title_x, header_y, &title_label, title_hstyle, area);
    header_rects.push((SortKey::Title, Rect::new(title_x, header_y, cols.title_w, 1)));
    if cols.author_w > 0 {
        let (author_label, author_active) = header_label("AUTHOR", SortKey::Author, sort);
        let author_hstyle = if author_active { story_header_active } else { story_header };
        draw_str_clipped(buf, author_x, header_y, &author_label, author_hstyle, area);
        header_rects.push((SortKey::Author, Rect::new(author_x, header_y, cols.author_w, 1)));
    }
    if cols.year_w > 0 {
        let (year_label, year_active) = header_label("YEAR", SortKey::Year, sort);
        let year_hstyle = if year_active { story_header_active } else { story_header };
        draw_str_clipped(buf, year_x, header_y, &year_label, year_hstyle, area);
        header_rects.push((SortKey::Year, Rect::new(year_x, header_y, cols.year_w, 1)));
    }
    if cols.rating_w > 0 {
        let (rating_label, rating_active) = header_label("RATING", SortKey::Rating, sort);
        let rating_hstyle = if rating_active { story_header_active } else { story_header };
        draw_str_clipped(buf, rating_x, header_y, &rating_label, rating_hstyle, area);
        header_rects.push((SortKey::Rating, Rect::new(rating_x, header_y, cols.rating_w, 1)));
    }
    // TYPE column header, above the interpreter labels in the right-hand zone.
    // Sortable like the other columns: active-styled with a direction arrow and
    // a header rect for click-to-sort.
    if badges_shown {
        let interp_hx = area.left() + row_w - 1 - cluster_w - COL_GAP - INTERP_COL_W;
        let (type_label, type_active) = header_label("TYPE", SortKey::Type, sort);
        let type_hstyle = if type_active { story_header_active } else { story_header };
        draw_str_clipped(buf, interp_hx, header_y, &type_label, type_hstyle, area);
        header_rects.push((SortKey::Type, Rect::new(interp_hx, header_y, INTERP_COL_W, 1)));
    }

    for (i, entry) in stories.iter().enumerate().skip(first).take(rows) {
        let y = list_top + (i - first) as u16;
        let row_rect = Rect::new(area.x, y, row_w, 1);
        row_rects.push((i, row_rect));
        let sel = i == selected;
        let style = if sel { dialog_button_active } else { dialog };
        for x in area.left()..area.left() + row_w {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(style);
            }
        }
        let marker = if sel { "▸ " } else { "  " };
        draw_str_clipped(buf, area.x, y, marker, style, row_rect);

        // A folder row is its name in the folder colour and nothing else: no
        // "(no metadata yet)", no year, no rating, `folder` for a type.
        let is_folder = entry.is_folder();
        let title_style = if sel || !is_folder { style } else { story_folder };
        let title_txt = truncate_to_width(&entry.title, cols.title_w as usize);
        draw_str_clipped(buf, title_x, y, &title_txt, title_style, row_rect);
        // While finding, a match can come from anywhere under the root, so its
        // folder rides after the title, muted; in a folder view every row's
        // folder is the one in the header and the label is `None`.
        if let Some(rel) = app::picker::folder_label(entry, heading.label_base()).filter(|_| !is_folder) {
            let used = UnicodeWidthStr::width(title_txt.as_str());
            let room = (cols.title_w as usize).saturating_sub(used + 2);
            if room >= 4 {
                let suffix = truncate_to_width(&format!("{rel}/"), room);
                let suffix_style = if sel { style } else { story_no_metadata };
                draw_str_clipped(buf, title_x + used as u16 + 2, y, &suffix, suffix_style, row_rect);
            }
        }

        if cols.author_w > 0 && !is_folder {
            let (author_txt, author_style) = match entry.meta.author.as_deref() {
                Some(a) if !a.is_empty() => {
                    (truncate_to_width(a, cols.author_w as usize), story_author)
                }
                _ => (
                    truncate_to_width("(no metadata yet)", cols.author_w as usize),
                    story_no_metadata,
                ),
            };
            // Selection highlight wins over the column's own color, same as
            // the title text above — the whole row reads as one bar.
            let author_style = if sel { style } else { author_style };
            draw_str_clipped(buf, author_x, y, &author_txt, author_style, row_rect);
        }

        if cols.year_w > 0 {
            if let Some(yr) = entry.meta.year.as_deref().filter(|s| !s.is_empty()) {
                let year_txt = truncate_to_width(yr, cols.year_w as usize);
                let year_style = if sel { style } else { story_year };
                draw_str_clipped(buf, year_x, y, &year_txt, year_style, row_rect);
            }
        }

        // IFDB's average rating to one decimal. Absent — unrated, or simply
        // never fetched — draws nothing at all: a blank cell, never "0.0",
        // which would read as a real (and damning) score.
        if cols.rating_w > 0 {
            if let Some(r) = entry.meta.ifdb_rating.filter(|r| r.is_finite()) {
                // The vote count rides beside the average so a 5.0 over three
                // ratings can't pass for a 5.0 over three hundred. A record with
                // an average but no count still shows the average alone.
                let cell = match entry.meta.ifdb_rating_count {
                    Some(n) => format!("{r:.1} ({n})"),
                    None => format!("{r:.1}"),
                };
                let rating_txt = truncate_to_width(&cell, cols.rating_w as usize);
                let rating_style = if sel { style } else { story_rating };
                draw_str_clipped(buf, rating_x, y, &rating_txt, rating_style, row_rect);
            }
        }

        // Right-hand zone: the TYPE column then the badge cluster
        // [blorb][save][hint]. No separators within the cluster, so present
        // badges stay vertically aligned across rows.
        let b = badges.get(i).copied().unwrap_or_default();
        if badges_shown {
            let bx = area.left() + row_w - 1 - cluster_w;
            // TYPE column, one gap to the left of the badges. Plain colour (not
            // the badge's reverse-block treatment); selection wins like the
            // other text columns.
            let interp_x = bx - COL_GAP - INTERP_COL_W;
            let interp_txt = if entry.is_folder() {
                "folder".to_string()
            } else {
                truncate_to_width(&interp_label(&entry.meta, b.blorb), INTERP_COL_W as usize)
            };
            let interp_style = if sel { style } else { story_badge };
            draw_str_clipped(buf, interp_x, y, &interp_txt, interp_style, row_rect);
            // Flags render as regular text like the other columns; the selection
            // bar wins over their own colour, same as title/author/year.
            let badge_style = if sel { style } else { story_badge };
            if b.save {
                draw_str_clipped(buf, bx, y, glyphs.save, badge_style, row_rect);
            }
            let hint_glyph = match b.hint {
                app::picker::HintBadge::Present => Some(glyphs.hint),
                app::picker::HintBadge::Available => Some(glyphs.hint_available),
                app::picker::HintBadge::None => None,
            };
            if let Some(g) = hint_glyph {
                draw_str_clipped(buf, bx + save_w, y, g, badge_style, row_rect);
            }
        }
    }

    if scrollbar_visible {
        let sb_area = Rect::new(area.right().saturating_sub(1), list_top, 1, rows as u16);
        app::render::scroll::draw_scrollbar(buf, sb_area, total, rows, list.target_offset(), scrollbar);
    }

    // Footer hint.
    let footer = build_footer(km, area.width, false);
    let fstyle = Style::new().fg(Color::DarkGray).patch(dialog);
    draw_str_clipped(buf, area.x, list_bottom, &footer, fstyle, area);

    (row_rects, rows, header_rects)
}

/// How long the gallery grid's scroll is considered "in motion" after the last
/// wheel notch or nav key (SQ-1213), mirroring `AppState::SIXEL_SCROLL_SETTLE_MS`
/// from the transcript's own debounce (SQ-1198, `crates/app/src/state.rs`): this
/// loop has no `AppState` to ride, so it tracks the identical 150ms window
/// locally instead — one default scroll-tween (120ms) plus a tick's margin.
const GALLERY_SCROLL_SETTLE_MS: u64 = 150;

/// True while the gallery grid's scroll is still "in motion" from a recent
/// wheel notch or nav key (SQ-1213) — see [`GALLERY_SCROLL_SETTLE_MS`].
fn gallery_scroll_in_motion(motion_at: Option<std::time::Instant>) -> bool {
    motion_at.is_some_and(|t| t.elapsed().as_millis() < GALLERY_SCROLL_SETTLE_MS as u128)
}

/// True while a gallery cover tile should render as its already-painted
/// letterbox footprint instead of building/placing a protocol (SQ-1213,
/// mirroring `sixel_scroll_suppress` in `render/inline_image.rs` for the
/// transcript's own debounce): sixel has no image ids, so re-placing a tile
/// mid-scroll re-sends its whole payload every frame, where kitty re-places an
/// existing upload by id for free. Kitty and half-blocks are untouched.
fn gallery_sixel_scroll_suppress(picker: &ratatui_image::picker::Picker, in_motion: bool) -> bool {
    picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel && in_motion
}

/// Draw the cover-gallery view (SQ-0374): a grid of story cover thumbnails, each
/// a fitted frontispiece over a one-row title caption, with the selected tile's
/// caption highlighted. `first_row` is the grid's scroll row (in/out — updated
/// to keep `selected` on screen). Returns each visible tile's `(index, rect)`
/// for click selection (the whole tile is the hit target), plus the resolved
/// column and visible-row counts the caller feeds back into navigation. Covers
/// paint only for tiles already decoded into `cover` AND already encoded into a
/// tile raster; anything else shows a plain letterbox until the async decoder
/// and `tiles`, the async ENCODER (SQ-1199), fill it in — this draw builds no
/// protocol of its own. `scroll_in_motion` gates the SQ-1213 sixel
/// scroll-settle debounce (see [`gallery_sixel_scroll_suppress`]).
#[allow(clippy::too_many_arguments)]
fn draw_story_gallery(
    stories: &[app::picker::StoryEntry],
    selected: usize,
    first_row: &mut usize,
    heading: &PickerHeading,
    cs: &app::colors::ColorScheme,
    km: &app::keymap::KeyMap,
    picker: Option<&ratatui_image::picker::Picker>,
    scroll_in_motion: bool,
    cover: &mut app::cover::CoverState,
    // The background tile encoder (SQ-1199): a visible tile whose raster isn't
    // built yet is REQUESTED here, never encoded on this thread.
    tiles: &mut app::cover::TileEncoder,
    // Where per-game directories live: a tile's cover is cached under the ROW's
    // key, which for one of several stories off a disk image is that story's own
    // directory (SQ-0859).
    data_base: &std::path::Path,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) -> (Vec<(usize, Rect)>, usize, usize) {
    use app::cover_gallery as g;
    let mut tile_rects: Vec<(usize, Rect)> = Vec::new();

    let dialog = cs.theme.get("dialog.background").style;
    let dialog_title = cs.theme.get("dialog.title").style;
    let story_tile_selected = cs.theme.get("story_tile_selected").style;
    let story_info_cover = cs.theme.get("story_info_cover").style;
    let story_tile = cs.theme.get("story_tile").style;
    let scrollbar = app::render::scroll::ScrollbarLook::from_theme(&cs.theme);

    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(dialog);
            }
        }
    }

    // Header (matches the list view's, with the toggle hint flipped).
    let header = heading.line(stories, "g: list");
    draw_str_clipped(buf, area.x, area.y, &header, dialog_title, area);

    // Grid region: below the header, above the footer row.
    let grid_top = area.y + 2;
    let grid_bottom = area.bottom().saturating_sub(1);
    if grid_bottom <= grid_top || area.width < g::TILE_W {
        return (tile_rects, 1, 1);
    }
    // Inset the grid one column so column-0 tiles still have a left gutter for
    // the selection frame (the other gutters come from the tile spacing).
    let grid = Rect::new(area.x + 1, grid_top, area.width.saturating_sub(1), grid_bottom - grid_top);
    let cols = g::columns(grid.width);
    let vis = g::visible_rows(grid.height);
    *first_row = g::scroll_to(selected, cols, vis, *first_row);
    let total = stories.len();
    let total_rows = total.div_ceil(cols);

    for vr in 0..vis {
        for col in 0..cols {
            let idx = (*first_row + vr) * cols + col;
            if idx >= total {
                continue;
            }
            let entry = &stories[idx];
            let tile = g::tile_rect(grid, col, vr);
            tile_rects.push((idx, tile));

            // Cover band = the whole tile, filled edge-to-edge with the fitted
            // image at maximum size (or, for a missing cover, the wrapped title
            // centred in it). The selection highlight is drawn afterwards in the
            // gutter ring around the tile, so it never shrinks the cover.
            let sel = idx == selected;
            let cover_rect = Rect::new(tile.x, tile.y, g::TILE_W, g::TILE_COVER_H);
            // Letterbox fill. When selected, fill the whole tile background with the
            // selection style so the bands around a centred (letterboxed) cover are
            // highlighted too — not only the gutter frame.
            let bg_style = if sel { story_tile_selected } else { story_info_cover };
            for y in cover_rect.top()..cover_rect.bottom() {
                for x in cover_rect.left()..cover_rect.right() {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_symbol(" ").set_style(bg_style);
                    }
                }
            }
            let mut drew_cover = false;
            if let Some(picker) = picker.filter(|_| !entry.is_folder()) {
                let key = entry.cover_key(data_base);
                if cover.has(&key) {
                    // Centre the cover in the tile via a self-computed fitted rect
                    // (image aspect + cell size), so it centres on both axes no
                    // matter how the render protocol reports its own size.
                    let fit = cover.fitted_tile_rect(picker, &key, cover_rect);
                    if gallery_sixel_scroll_suppress(picker, scroll_in_motion) {
                        // SQ-1213: mid-scroll under sixel, leave the tile as the
                        // letterbox footprint already filled above rather than
                        // rebuilding/re-placing its whole payload this frame.
                        // Nothing is requested either (SQ-1199): a frame that has
                        // decided not to show a payload has no use for one, and a
                        // fling would otherwise queue a row of encodes per notch
                        // for rasters no suppressed frame will place.
                        drew_cover = true;
                    } else {
                        let tkey = app::cover::TileKey::new(&key, fit, picker);
                        if let Some(proto) = cover.tile(&tkey) {
                            let id = app::render::graphics::place_protocol(proto, fit, buf);
                            cover.note_tile_placed(id);
                            drew_cover = true;
                        } else if let Some(img) =
                            cover.image(&key).filter(|_| !tiles.failed(&tkey))
                        {
                            // SQ-1199: the resize + protocol encode goes to the
                            // background encoder and the tile keeps the letterbox
                            // footprint already filled above until it lands — the
                            // draw never blocks on one. `request` dedupes, so the
                            // 16ms tick that keeps redrawing while tiles are
                            // pending queues each tile exactly once.
                            //
                            // The footprint, not the titled placeholder: this
                            // story HAS a cover, and flashing the no-cover box for
                            // the frame or two before its raster lands would read
                            // as a glitch. A cover whose encode actually FAILED is
                            // `failed()` above and does fall through to it.
                            tiles.request(tkey, img, picker);
                            drew_cover = true;
                        }
                    }
                }
            }
            if !drew_cover {
                // No cover art: draw a simple placeholder — a border around the tile
                // with the wrapped title centred inside it. Selected tiles keep the
                // selection background (filled above) so it sits on the highlight.
                let title_style = if sel { story_tile_selected } else { story_tile };
                if !sel {
                    for y in cover_rect.top()..cover_rect.bottom() {
                        for x in cover_rect.left()..cover_rect.right() {
                            if let Some(c) = buf.cell_mut((x, y)) {
                                c.set_symbol(" ").set_style(title_style);
                            }
                        }
                    }
                }
                // Border ring around the placeholder.
                let (x0, x1) = (cover_rect.left(), cover_rect.right().saturating_sub(1));
                let (y0, y1) = (cover_rect.top(), cover_rect.bottom().saturating_sub(1));
                for x in x0..=x1 {
                    if let Some(c) = buf.cell_mut((x, y0)) { c.set_symbol("─").set_style(title_style); }
                    if let Some(c) = buf.cell_mut((x, y1)) { c.set_symbol("─").set_style(title_style); }
                }
                for y in y0..=y1 {
                    if let Some(c) = buf.cell_mut((x0, y)) { c.set_symbol("│").set_style(title_style); }
                    if let Some(c) = buf.cell_mut((x1, y)) { c.set_symbol("│").set_style(title_style); }
                }
                for (px, py, ch) in [(x0, y0, "┌"), (x1, y0, "┐"), (x0, y1, "└"), (x1, y1, "┘")] {
                    if let Some(c) = buf.cell_mut((px, py)) {
                        c.set_symbol(ch).set_style(title_style);
                    }
                }
                // Title centred inside the border.
                let inner_w = g::TILE_W.saturating_sub(2);
                let inner_h = g::TILE_COVER_H.saturating_sub(2);
                let lines = wrap_to_width(&entry.title, inner_w as usize);
                let shown = (lines.len() as u16).min(inner_h);
                let start_y = cover_rect.y + 1 + (inner_h - shown) / 2;
                for (i, line) in lines.iter().take(shown as usize).enumerate() {
                    let lw = UnicodeWidthStr::width(line.as_str()) as u16;
                    let x = cover_rect.x + 1 + inner_w.saturating_sub(lw) / 2;
                    draw_str_clipped(buf, x, start_y + i as u16, line, title_style, cover_rect);
                }
            }

            // Selection highlight: fill the one-cell gutter ring around the tile
            // with the selection style, framing the cover without shrinking it.
            // Clipped to the grid, so a tile against an edge frames only on the
            // sides that have a gutter.
            if sel {
                let sfx = story_tile_selected;
                let left = tile.x as i32 - 1;
                let right = tile.x as i32 + g::TILE_W as i32;
                let top = tile.y as i32 - 1;
                let bottom = tile.y as i32 + g::TILE_COVER_H as i32;
                let mut frame = |x: i32, y: i32| {
                    if x >= area.x as i32
                        && x < area.right() as i32
                        && y > area.y as i32
                        && y < grid_bottom as i32
                    {
                        if let Some(c) = buf.cell_mut((x as u16, y as u16)) {
                            c.set_symbol(" ").set_style(sfx);
                        }
                    }
                };
                for x in left..=right {
                    frame(x, top);
                    frame(x, bottom);
                }
                for y in top..=bottom {
                    frame(left, y);
                    frame(right, y);
                }
            }
        }
    }

    // Scrollbar in the spare width to the grid's right when the grid overflows.
    if total_rows > vis {
        let sb_area = Rect::new(area.right().saturating_sub(1), grid.y, 1, grid.height);
        app::render::scroll::draw_scrollbar(buf, sb_area, total_rows, vis, *first_row, scrollbar);
    }

    // Footer hint, from the same registry-driven hints the list footer uses
    // (SQ-0796) — the same line and the same drop order (SQ-1227), with `g`
    // naming where it goes (`list`) rather than where it is.
    let footer = build_footer(km, area.width, true);
    let fstyle = ratatui::style::Style::new()
        .fg(ratatui::style::Color::DarkGray)
        .patch(dialog);
    let footer_txt = truncate_to_width(&footer, area.width as usize);
    draw_str_clipped(buf, area.x, grid_bottom, &footer_txt, fstyle, area);

    (tile_rects, cols, vis)
}

/// Carry out a [`ModalAction`] from the IFDB search modal (SQ-0413): translate
/// it into a worker job (search/resolve/download), open a browser page, or close
/// the modal. The picker owns the worker and the download directory, so the
/// modal itself stays network- and filesystem-free.
fn dispatch_search_action(
    action: app::ifdb_search_modal::ModalAction,
    worker: &app::ifdb_search::SearchWorker,
    dir: &std::path::Path,
    modal: &mut Option<app::ifdb_search_modal::SearchModal>,
) {
    use app::ifdb_search::SearchJob;
    use app::ifdb_search_modal::ModalAction;
    match action {
        ModalAction::None => {}
        ModalAction::Close => *modal = None,
        ModalAction::Search(q) => worker.request(SearchJob::Search(q)),
        ModalAction::Resolve(tuid) => worker.request(SearchJob::Resolve(tuid)),
        ModalAction::Download(url) => {
            // SQ-0474: the iFiction record resolved alongside this game's
            // download options rides along so the worker can populate the
            // sidecar + cover once the file lands — taken here, once, so a
            // later Resolve for a different game doesn't reuse it.
            let record = modal.as_mut().and_then(|m| m.take_pending_record());
            worker.request(SearchJob::Download { url, dest: dir.to_path_buf(), record })
        }
        ModalAction::OpenInBrowser(url) => open_url(&url),
        ModalAction::Seed => worker.request(SearchJob::Seed),
    }
}

/// Open a URL in the user's default browser. The picker holds mouse capture
/// (for click-to-select), so the terminal never handles a plain click on an
/// OSC 8 hyperlink (SQ-0367) — we open it ourselves instead. Fire-and-forget:
/// the URL is passed as a single argument (no shell), so it needs no escaping.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    let _ = cmd
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Draw the highlighted story's metadata panel: title, filesystem info,
/// format/version/release, serial (Z only), IFID, present features, bundled
/// resources (self-blorb or an associated sibling blorb), and saves. Pure
/// renderer — no state, no interaction (the picker loop wires toggling/
/// slide/lazy-resolve). `link_rects` is cleared and refilled with the screen
/// rect + full URL of every rendered OSC 8 link, so the loop can open one on a
/// click (mouse capture keeps the terminal from doing it — SQ-0367).
/// Format an RFC3339 save timestamp (`YYYY-MM-DDTHH:MM:SSZ`) as `YYYY-MM-DD HH:MM`
/// — date plus time-of-day, so same-day saves are distinguishable (SQ-0411). Falls
/// back to the leading date, or the raw string, when it isn't the expected shape.
fn save_when(saved_at: &str) -> String {
    match saved_at.get(0..16) {
        Some(s) if s.as_bytes().get(10) == Some(&b'T') => s.replacen('T', " ", 1),
        _ => saved_at.get(0..10).unwrap_or(saved_at).to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_info_panel(
    title: &str,
    filename: &str,
    meta: &app::picker::StoryMeta,
    aux: Option<&app::picker::StoryAux>,
    scroll: usize,
    area: Rect,
    picker: Option<&ratatui_image::picker::Picker>,
    cover: &mut app::cover::CoverState,
    entry_path: &std::path::Path,
    // What this ROW's cover is cached under — its own path for a loose story,
    // its game directory for one of several stories off a disk image, which is
    // the only thing that keeps five games on one image from sharing a jacket
    // (SQ-0859). See `app::picker::StoryEntry::cover_key`.
    cover_key: &std::path::Path,
    animating: bool,
    hint_sidecar: Option<&std::path::Path>,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
    link_rects: &mut Vec<(Rect, String)>,
    resource_rects: &mut Vec<(Rect, ResourceRef)>,
) -> usize {
    link_rects.clear();
    resource_rects.clear();
    if area.width < 2 || area.height < 2 {
        return 0;
    }
    let story_info = cs.theme.get("story_info").style;
    let story_info_title = cs.theme.get("story_info_title").style;
    let story_info_value = cs.theme.get("story_info_value").style;
    let story_info_label = cs.theme.get("story_info_label").style;
    let story_info_blurb = cs.theme.get("story_info_blurb").style;
    let story_info_link = cs.theme.get("story_info_link").style;
    let story_info_continuation = cs.theme.get("story_info_continuation").style;
    let story_info_cover = cs.theme.get("story_info_cover").style;
    let story_info_artwork = cs.theme.get("story_info_artwork").style;
    let story_info_artwork_active = cs.theme.get("story_info_artwork:active").style;
    let scrollbar = app::render::scroll::ScrollbarLook::from_theme(&cs.theme);
    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(story_info);
            }
        }
    }

    // Framed box with a centered, bracketed "Info" title (chrome via draw_panel).
    let info_segs = [InsetSegment { text: "Info", active: false }];
    let frame = draw_panel(
        buf,
        &PanelSpec {
            area,
            border_selector: "panel.border",
            border_color: Some(story_info),
            border_style: None,
            glyphs: &PaneGlyphs::default(),
            header_on: true,
            strip: Some(PanelStrip {
                segments: &info_segs,
                base: story_info_title,
                active: story_info_title,
            }),
            body_fill: None,
        },
        &cs.theme,
    );

    let mut inner = frame.content;

    // Cover band: top of the panel, ≤50% of the panel's inner height is the
    // *maximum* fit box; the actual band is sized down to the image's
    // aspect-fitted height so no dead letterbox rows push the info text down.
    // Only drawn when the selected story has a decoded frontispiece and a
    // picker exists.
    if let Some(picker) = picker {
        if cover.has(cover_key) {
            let cover_h = (inner.height / 2).min(inner.height.saturating_sub(1));
            if cover_h >= 1 {
                let cover_area = Rect::new(inner.x, inner.y, inner.width, cover_h);
                let mut used_h = 0u16;
                if let Some(proto) = cover.protocol(picker, cover_key, cover_area, animating) {
                    // Fitted (aspect-preserved) size, clamped to the max box.
                    let sz = proto.size();
                    let used_w = sz.width.min(inner.width);
                    used_h = sz.height.min(cover_h);
                    // Themed letterbox fill, sized to the actual fitted band
                    // (not the full max box) so there's no dead space below.
                    let fill_area = Rect::new(cover_area.x, cover_area.y, cover_area.width, used_h);
                    for y in fill_area.top()..fill_area.bottom() {
                        for x in fill_area.left()..fill_area.right() {
                            if let Some(c) = buf.cell_mut((x, y)) {
                                c.set_symbol(" ").set_style(story_info_cover);
                            }
                        }
                    }
                    // Top-aligned, horizontally centered within the band.
                    let dest = Rect::new(
                        cover_area.x + (inner.width - used_w) / 2,
                        cover_area.y,
                        used_w,
                        used_h,
                    );
                    let id = app::render::graphics::place_protocol(proto, dest, buf);
                    cover.note_proto_placed(id);
                }
                if used_h > 0 {
                    inner = Rect::new(inner.x, inner.y + used_h, inner.width, inner.height - used_h);
                }
            }
        }
    }

    let mut lines: Vec<(String, ratatui::style::Style)> = Vec::new();
    // Line index → full URL for lines that should render as OSC 8 hyperlinks
    // (SQ-0367): the visible text may truncate, but the whole visible label
    // stays clickable and opens the full URL.
    let mut link_urls: Vec<(usize, String)> = Vec::new();
    // Line index → previewable resource (SQ-0347), for the Pict/Snd rows below.
    let mut resource_refs: Vec<(usize, ResourceRef)> = Vec::new();

    // Title.
    lines.push((title.to_string(), story_info_title));
    // The filename gets a line to itself, and the sizes get the next one. A
    // compilation's name is long enough on its own — `Lost Treasures of Infocom,
    // The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg:LEATHRGODDESSES` — that
    // anything sharing its line only wraps away from the thing it belongs to.
    //
    // A story off a compilation names itself as the disk names it (SQ-0859):
    // several rows share one filename, and the suffix is what says which game of
    // them this row is.
    let container = match &meta.disk_entry {
        Some(name) => format!("{filename}:{name}"),
        None => filename.to_string(),
    };
    lines.push((container, story_info_value));
    // The second size appears only when the file on disk is a container, because
    // then the first one measures the container and not the game (SQ-0771): an
    // Amiga `.adf` is 880 KB whatever it holds, and a blorb/zip carries resources
    // beside the executable.
    //
    // No mtime. It dates the FILE, which is when this copy was written to this
    // disk — it says nothing about when the game was published, and next to a
    // release and serial that do, it invited exactly that misreading.
    let mut size_line = human_size(meta.size_bytes);
    if meta.story_bytes > 0 && meta.story_bytes != meta.size_bytes {
        size_line.push_str(&format!(" · story {}", human_size(meta.story_bytes)));
    }
    lines.push((size_line, story_info_value));
    // format + version · release.
    let mut fmt_line = meta.format.clone();
    if let Some(v) = &meta.version {
        fmt_line = match meta.engine {
            app::picker::Engine::ZCode => format!("{} v{}", meta.format, v),
            app::picker::Engine::Glulx | app::picker::Engine::Scott => {
                format!("{} {}", meta.format, v)
            }
        };
    }
    if let Some(r) = meta.release {
        fmt_line.push_str(&format!(" · Release {r}"));
    }
    lines.push((fmt_line, story_info_value));
    // serial (Z only).
    if let Some(s) = &meta.serial {
        lines.push((format!("Serial {s}"), story_info_value));
    }
    // ifid.
    lines.push((format!("IFID {}", meta.ifid), story_info_value));
    // Associated resource blorb (SQ-0372): a resource .blorb stored beside the
    // story (e.g. Lurking.blb, beyondzork.blb). Named here, up-front, so it is
    // visible without scrolling to the Resources section below. Only the sidecar
    // case — a self-contained blorb's resources live in the story file itself.
    if let Some((src, _)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
        if let Some(name) = src.file_name().and_then(|n| n.to_str()) {
            lines.push((format!("Resource blorb: {name}"), story_info_value));
        }
    }
    // Associated hint sidecar (SQ-0443): an InvisiClues/hint image detected
    // beside the story and hidden from the list. Named here so the player sees
    // hints are available and which file supplies them. With no local file, note
    // when a matching InvisiClues can be downloaded with `H` (SQ-0445).
    if let Some(name) = hint_sidecar.and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
        lines.push((format!("Hints: {name}"), story_info_value));
    } else {
        let stem = std::path::Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if app::hints::hint_download_for(&meta.ifid, stem, title).is_some() {
            lines.push(("Hints: available to download (press H)".to_string(), story_info_value));
        }
    }
    // author · year · genre (SQ-0348): one line, present parts only — a story
    // with none of the three renders no line at all, so a no-metadata panel
    // is unchanged from before this field existed.
    let meta_bits: Vec<&str> = [meta.author.as_deref(), meta.year.as_deref(), meta.genre.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !meta_bits.is_empty() {
        lines.push((meta_bits.join(" · "), story_info_value));
    }
    // blurb (SQ-0348): word-wrapped to the panel's content width, each
    // wrapped row pushed as its own entry so it rides the same
    // scroll/overflow accounting as every other panel line below. Split on the
    // paragraph breaks html_to_text left in, wrapping each independently so a
    // `<br/>` in the source stays a visible break rather than collapsing.
    //
    // Reserve the scrollbar's gutter column when wrapping: whether the panel
    // overflows depends on the wrapped line count, so it can't be known here —
    // always wrap to the narrower width so a wrapped line is never clipped by
    // (or drawn under) the scrollbar in the overflow case.
    let wrap_w = (inner.width as usize).saturating_sub(1);
    if let Some(desc) = meta.description.as_deref().filter(|s| !s.is_empty()) {
        for para in desc.lines() {
            if para.trim().is_empty() {
                lines.push((String::new(), story_info_blurb));
            } else {
                for row in wrap_to_width(para, wrap_w) {
                    lines.push((row, story_info_blurb));
                }
            }
        }
    }
    // IFDB page link (SQ-0348): a real OSC 8 hyperlink (SQ-0367) so the visible
    // text is clickable even when the URL truncates; only present once fetched.
    if let Some(link) = meta.ifdb_link.as_deref().filter(|s| !s.is_empty()) {
        link_urls.push((lines.len(), link.to_string()));
        lines.push((format!("IFDB: {link}"), story_info_link));
    } else if meta.fetch_not_found {
        // A fetch ran but IFDB had no record for this IFID (common for Infocom
        // releases IFDB indexes under a different IFID). Offer a manual search
        // by title so the user isn't at a dead end (SQ-0371).
        let url = app::ifdb::search_url(title);
        link_urls.push((lines.len(), url.clone()));
        lines.push((format!("IFDB search: {url}"), story_info_link));
    }
    // features line (present badges only).
    let feats = feature_words(&meta.features, aux);
    if !feats.is_empty() {
        lines.push((format!("Features: {}", feats.join(" ")), story_info_value));
    }

    // Detected picture archives (SQ-0789). Read-only inventory: what art this
    // story has beside it, and which of it is actually in force. The list comes
    // from `discover_art_candidates`, the *same* function the launch-options
    // dialog offers, so the panel and the dialog cannot drift into two answers.
    //
    // Display-only, and it must stay that way: enumerating candidates is safe
    // because a person reads the list and supplies the pairing the file format
    // cannot (no release number, no serial). Nothing here may ever be fed into
    // `PictureOverride::resolve` — that is the auto-pairing SQ-0734 rejected,
    // whose failure mode is Arthur's plates drawn into Zork Zero with nothing on
    // screen to say so.
    if let Some(a) =
        aux.filter(|a| !a.art_candidates.is_empty() || a.art_in_use.is_some())
    {
        lines.push((String::new(), story_info_value));
        lines.push((
            format!("Artwork · {} detected for this story", a.art_candidates.len()),
            story_info_label,
        ));
        for c in &a.art_candidates {
            let in_use = a.art_in_use.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&c.filename));
            // The dialog's columns, minus what a read-only row cannot use: the
            // multi-part note appears only when there is one, because a panel
            // with other content to show cannot spend a column on a constant.
            let mut row = format!(" {:<13} {:<8} {} pictures", c.filename, c.rendition, c.pictures);
            let note = app::launch_options::parts_note(c);
            if !note.is_empty() {
                row.push_str(&format!(" · {}", note.trim_start()));
            }
            // An archive on the volume the story was mounted out of has no path
            // of its own; saying so keeps the panel from listing a file the
            // folder plainly does not hold (SQ-0843). WHICH volume, since an
            // archive may come off a sibling of the disk booted (SQ-0862/0865),
            // and in the dialog's words — one phrase, so the panel and the
            // dialog cannot disagree about one archive.
            let where_note = app::launch_options::medium_note(c);
            if !where_note.is_empty() {
                row.push_str(&format!(" · {where_note}"));
            }
            if in_use {
                row.push_str("  ← in use");
            }
            lines.push((row, if in_use { story_info_artwork_active } else { story_info_artwork }));
        }
        // A named archive that is not among the detected ones — an absolute
        // path, or a file under an unrelated name like the renamed FMVPOKER.EG1
        // — is still what the game will draw. Saying so keeps the block honest:
        // the list above is a name guess, the config key is an instruction.
        if let Some(name) = a.art_in_use.as_deref().filter(|n| {
            !a.art_candidates.iter().any(|c| c.filename.eq_ignore_ascii_case(n))
        }) {
            lines.push((format!(" in use: {name}"), story_info_artwork_active));
        }
    }

    // Saves + sidecars (SQ-0285). Rendered above Resources so the user's own
    // saves are the first thing they see below the metadata.
    if let Some(a) = aux {
        let has_any = !a.saves.is_empty() || !a.qzl_saves.is_empty()
            || !a.auto_saves.is_empty() || !a.sidecars.is_empty();
        if has_any {
            lines.push((String::new(), story_info_value));
            // Header: "Saves · <dir>" with $HOME abbreviated to ~.
            let dir = abbreviate_home(&a.game_dir);
            lines.push((format!("Saves · {dir}"), story_info_label));
            for s in &a.saves {
                let when = save_when(&s.saved_at);
                let fname = s.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Save summary (SQ-0411): name · location · score, then turn/date/file.
                let mut summary = s.name.clone();
                if let Some(loc) = &s.location {
                    summary.push_str(" · ");
                    summary.push_str(loc);
                }
                if let Some(score) = s.score {
                    summary.push_str(&format!(" · score {score}"));
                }
                lines.push((format!(" {}  turn {} · {}  {}", summary, s.turns, when, fname), story_info_value));
            }
            for q in &a.qzl_saves {
                let when = save_when(&q.saved_at);
                let fname = q.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                lines.push((format!(" {}  {}  {}", q.name, when, fname), story_info_value));
            }
            if !a.auto_saves.is_empty() {
                lines.push(("Automatic:".to_string(), story_info_label));
                for q in &a.auto_saves {
                    let when = save_when(&q.saved_at);
                    let fname = q.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    lines.push((format!(" (auto) {}  {}  {}", q.name, when, fname), story_info_value));
                }
            }
            if !a.sidecars.is_empty() {
                lines.push((format!("Sidecars: {}", a.sidecars.join(" · ")), story_info_value));
            }
        }
    }

    // Resources: self_blorb (the story is itself a blorb), else aux.assoc_blorb
    // (a sidecar). `blorb_path` is where a clicked resource re-reads its bytes.
    let (res_header, chunks, blorb_path): (Option<String>, &[app::picker::ChunkInfo], Option<std::path::PathBuf>) =
        if let Some(c) = &meta.self_blorb {
            (Some(format!("Resources ({filename})")), c.as_slice(), Some(entry_path.to_path_buf()))
        } else if let Some((p, c)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
            // The sidecar filename is named up-front in the metadata block, so
            // this header stays generic rather than repeating it.
            (Some("Resources".to_string()), c.as_slice(), Some(p.clone()))
        } else {
            (None, &[], None)
        };
    if let Some(h) = res_header {
        lines.push((String::new(), story_info_value));
        lines.push((h, story_info_label));
        for c in chunks {
            let base = format!(
                " #{}  {} — {}",
                c.number,
                resource_usage_label(&c.usage),
                resource_type_label(&c.chunk_type),
            );
            let line = match &c.detail {
                Some(d) => format!("{base} · {d} ({})", human_size(c.len as u64)),
                None => format!("{base} ({})", human_size(c.len as u64)),
            };
            // Images (Pict) and sounds (Snd) are clickable to preview (SQ-0347):
            // style them like a link and map their line to the resource so a
            // click can re-read the bytes and pop the modal.
            let kind = match c.usage.trim() {
                "Pict" => Some(PreviewKind::Image),
                "Snd" => Some(PreviewKind::Sound),
                _ => None,
            };
            match (kind, &blorb_path) {
                (Some(kind), Some(bp)) => {
                    resource_refs.push((
                        lines.len(),
                        ResourceRef {
                            blorb_path: bp.clone(),
                            kind,
                            number: c.number,
                            label: format!("{} #{}", resource_usage_label(&c.usage), c.number),
                        },
                    ));
                    lines.push((line, story_info_link));
                }
                _ => lines.push((line, story_info_value)),
            }
        }
    }

    // Sounds the MEDIUM carries (SQ-0907), which no Blorb block above can show: the
    // two Infocom games that use sound ship it on the release disk as an
    // Infocom-native container, not as `Snd ` resources. Listed with the sample's own
    // name because that is what a person recognises — Sherlock's are `armor`,
    // `growl`, `splash` — and with the rate the disk states.
    if let Some(a) = aux {
        if !a.disk_sounds.is_empty() {
            lines.push((String::new(), story_info_value));
            lines.push((format!("Sound on the medium ({})", a.disk_sounds.len()), story_info_label));
            for s in &a.disk_sounds {
                lines.push((
                    format!(
                        " #{}  {} — {} Hz · {} ({})",
                        s.effect,
                        s.name,
                        s.rate,
                        human_size(s.frames as u64),
                        // Effects 11, 12 and 13 of Sherlock are all `heart` at
                        // different pitches, so the sample name alone can repeat.
                        "8-bit mono",
                    ),
                    story_info_value,
                ));
            }
        }
    }

    // Typefaces the MEDIUM carries (SQ-1018), which no Blorb block above can
    // show: an Infocom Macintosh release keeps its faces in the application's
    // resource fork, an Amiga one keeps a disk font beside the story. Listed with
    // the cell each is drawn for, and with which one the v6 raster path actually
    // takes — because "present but unused" is the exact shape of SQ-1018, where
    // the Masterpieces CD carried FONT 524 for every graphical game on it and the
    // renderer reached none of them. That cost a bug report; here it costs a
    // glance.
    if let Some(a) = aux {
        if !a.disk_fonts.is_empty() {
            lines.push((String::new(), story_info_value));
            lines.push((
                format!("Fonts on the medium ({})", a.disk_fonts.len()),
                story_info_label,
            ));
            for f in &a.disk_fonts {
                let mut row = format!(" {}  {}x{}", f.name, f.width, f.height);
                // Proportional is worth saying because it is why a face can be
                // present and still not be the one drawn (SQ-0916).
                if f.proportional {
                    row.push_str(" · proportional");
                }
                if f.used {
                    row.push_str(" · in use");
                }
                lines.push((row, story_info_value));
            }
        }

        // Typefaces the USER'S OWN disks under `~/.lanthorn/` carry (SQ-1038) —
        // a Workbench or System disk kept beside the stories rather than any one
        // game's release. Same shape as the block above, minus "in use": nothing
        // renders with one of these yet (SQ-1037), so every row here is simply
        // present. Only ever populated for a Version 6 story on this disk's own
        // machine — `picker::aux_for` decides that, not this.
        //
        // **Grouped by disk, disk named once.** A system disk carries a whole
        // drawer — eighteen faces off a System 6.0.8 startup disk, fourteen off a
        // Workbench floppy — and repeating a sixty-character filename on every one
        // of them buries the faces in their own provenance. The disk still has to
        // be named, because Workbench 1.2 and 1.3 ship IDENTICAL font drawers and
        // would otherwise read as one list silently standing for two; naming it
        // once as a heading says it without drowning the rows under it.
        if !a.system_fonts.is_empty() {
            lines.push((String::new(), story_info_value));
            lines.push((format!("System fonts ({})", a.system_fonts.len()), story_info_label));
            // Grouped in FIRST-APPEARANCE order rather than sorted, so the list
            // reads in the order the directory scan found the disks and does not
            // reshuffle when a face is added to one of them.
            let mut seen: Vec<&str> = Vec::new();
            for f in &a.system_fonts {
                if !seen.contains(&f.disk.as_str()) {
                    seen.push(f.disk.as_str());
                }
            }
            for disk in seen {
                let n = a.system_fonts.iter().filter(|f| f.disk == disk).count();
                lines.push((format!(" {disk} ({n})"), story_info_value));
                for f in a.system_fonts.iter().filter(|f| f.disk == disk) {
                    let mut row = format!("   {}  {}x{}", f.name, f.width, f.height);
                    if f.proportional {
                        row.push_str(" · proportional");
                    }
                    lines.push((row, story_info_value));
                }
            }
        }
    }

    // Wrap every logical line to the panel's content width (SQ-0861). One row
    // per line clipped anything wider than the panel — the file line the report
    // named, but equally the IFID, `Saves · <dir>`, `Sidecars:`, and every save,
    // artwork and resource row that ends in a filename.
    //
    // Whether the scrollbar's gutter column is spent is itself a function of the
    // wrapped row count, so it can't be known before wrapping. Wrap at the full
    // width first; only if THAT already overflows is the narrower width used,
    // and narrowing can only add rows, so the second pass cannot un-overflow.
    // Two passes at most, and a panel that fits is laid out exactly as before.
    let wrap_rows = |w: u16| -> Vec<PanelRow> {
        let first_w = w as usize;
        // Too narrow to spend two columns on an indent: wrap flush instead of
        // refusing to wrap. `indent == 0` is also what suppresses the marker.
        let indent = if first_w > PANEL_CONT_INDENT { PANEL_CONT_INDENT } else { 0 };
        let mut out = Vec::with_capacity(lines.len());
        for (li, (text, style)) in lines.iter().enumerate() {
            for (ri, row) in wrap_panel_line(text, first_w, first_w - indent).into_iter().enumerate() {
                out.push(PanelRow { text: row, style: *style, cont: ri > 0, src: li });
            }
        }
        out
    };
    let mut rows = wrap_rows(inner.width);
    // Reserve a 1-col gutter for the scrollbar when content overflows.
    let overflow = rows.len() as u16 > inner.height;
    let text_area = if overflow {
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(1), inner.height)
    } else {
        inner
    };
    if overflow {
        rows = wrap_rows(text_area.width);
    }
    let cont_indent = if text_area.width as usize > PANEL_CONT_INDENT { PANEL_CONT_INDENT as u16 } else { 0 };
    let content_height = inner.height as usize;
    let max_scroll = rows.len().saturating_sub(content_height);
    let eff = scroll.min(max_scroll);
    let end = (eff + content_height).min(rows.len());
    for (vi, row) in rows[eff..end].iter().enumerate() {
        let (text, style) = (&row.text, &row.style);
        let li = row.src;
        let y = inner.y + vi as u16;
        // A continuation is set in from the panel edge behind its own marker, so
        // a wrapped tail reads as more of the field above rather than as a new
        // one. The marker carries `story_info_continuation`; the text keeps the
        // style of the logical line it belongs to.
        let row_area = if row.cont {
            draw_str_clipped(buf, text_area.x, y, PANEL_CONT_MARK, story_info_continuation, text_area);
            Rect::new(
                text_area.x + cont_indent,
                text_area.y,
                text_area.width.saturating_sub(cont_indent),
                text_area.height,
            )
        } else {
            text_area
        };
        if let Some((_, url)) = link_urls.iter().find(|(idx, _)| *idx == li) {
            // OSC 8 hyperlink (SQ-0367): the whole visible label is clickable and
            // opens the full URL, so a truncated URL still works. Degrades to
            // plain styled text on terminals without hyperlink support.
            let rect = Rect::new(row_area.x, y, row_area.width, 1);
            let link = hyperrat::Link::new(text.as_str(), url.as_str()).style(*style);
            ratatui::widgets::Widget::render(link, rect, buf);
            // hyperrat packs the whole OSC 8 escape sequence into the first
            // cell's symbol but leaves its diff option at None, so ratatui's diff
            // measures the escape bytes as display width (huge) and then skips
            // that many following cells when flushing — leaving a stale label
            // tail and a corrupted scrollbar on this row. Pin the cell to width 1
            // (ratatui's documented remedy for escape-symbol cells); hyperrat's
            // own Skip cells already carry the label's real column span.
            if let Some(first) = buf.cell_mut(ratatui::layout::Position::new(rect.x, rect.y)) {
                first.set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
                    std::num::NonZeroU16::new(1).unwrap(),
                ));
            }
            link_rects.push((rect, url.clone()));
            continue;
        }
        if let Some((_, rref)) = resource_refs.iter().find(|(idx, _)| *idx == li) {
            // A previewable Pict/Snd row (SQ-0347): draw it, and record its rect
            // so a click can open the resource preview modal.
            draw_str_clipped(buf, row_area.x, y, text, *style, row_area);
            resource_rects.push((Rect::new(row_area.x, y, row_area.width, 1), rref.clone()));
            continue;
        }
        draw_str_clipped(buf, row_area.x, y, text, *style, row_area);
    }
    if overflow {
        let sb_area = Rect::new(inner.right().saturating_sub(1), inner.y, 1, inner.height);
        app::render::scroll::draw_scrollbar(buf, sb_area, rows.len(), inner.height as usize, eff, scrollbar);
    }
    max_scroll
}

/// Load a clicked resource into a preview (SQ-0347). For an image, decode its
/// `Pict` bytes; for a sound, play it once through `audio` (constructed lazily
/// on first use, then held by the caller). Always returns a preview — an
/// unreadable blorb, an undecodable image, or a silent audio backend surfaces
/// as a status line rather than nothing.
fn open_resource_preview(
    rref: &ResourceRef,
    audio: &mut Option<audio::AudioBackend>,
    volume: u8,
) -> ResourcePreview {
    let blorb = std::fs::read(&rref.blorb_path)
        .ok()
        .and_then(|bytes| blorb::Blorb::parse(bytes).ok());
    match rref.kind {
        PreviewKind::Image => {
            let image = blorb
                .as_ref()
                .and_then(|b| b.resource(b"Pict", rref.number))
                .and_then(|(_, data)| app::cover::decode(data));
            let status = image
                .is_none()
                .then(|| "Can't preview this image (unsupported format).".to_string());
            ResourcePreview { title: rref.label.clone(), image, proto: None, status, zoom: PreviewZoom::Fit }
        }
        PreviewKind::Sound => {
            let status = play_preview_sound(blorb.as_ref(), rref.number, audio, volume);
            ResourcePreview {
                title: rref.label.clone(), image: None, proto: None, status: Some(status),
                zoom: PreviewZoom::Fit,
            }
        }
    }
}

/// Play sound resource `number` from `blorb` once, returning the status line for
/// the modal. Lazily constructs the audio backend into `audio` on first use.
fn play_preview_sound(
    blorb: Option<&blorb::Blorb>,
    number: u32,
    audio: &mut Option<audio::AudioBackend>,
    volume: u8,
) -> String {
    let Some(blorb) = blorb else {
        return "Couldn't read the resource blorb.".to_string();
    };
    let Some((bytes, kind)) = blorb.sound(number) else {
        return "Sound resource not found.".to_string();
    };
    let Some(fmt) = app::state::sound_kind_to_format(kind) else {
        return "Unsupported sound format.".to_string();
    };
    let backend = audio.get_or_insert_with(|| audio::AudioBackend::new(volume));
    match backend.play_sample(bytes, fmt, 8, 1) {
        Some(_) => "Playing…   (Esc / Enter to close)".to_string(),
        None => "No audio output available.".to_string(),
    }
}

/// Draw the resource-preview modal (SQ-0347) centred over `area`: dialog chrome
/// (border, title, ✕ close, a Close button) with the fitted-or-zoomed image
/// (SQ-0486) — or a status line — in its content rect. Returns the dialog's hit
/// rects for dismissal.
fn draw_resource_preview(
    pv: &mut ResourcePreview,
    area: Rect,
    picker: Option<&ratatui_image::picker::Picker>,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
    // Shares `cover`'s delete queue (SQ-1190) — see the doc comment on
    // `ResourcePreview::proto`.
    cover: &mut app::cover::CoverState,
) -> app::render::dialog::DialogRects {
    use app::render::dialog::{draw_dialog, ButtonId, DialogButton, DialogSpec, DialogStyle, Placement};
    // Centre a generous box: 80% of the terminal, floored so it stays usable on
    // small screens and never exceeds the area.
    let w = ((area.width as u32 * 4 / 5) as u16).clamp(20, area.width).max(1);
    let h = ((area.height as u32 * 4 / 5) as u16).clamp(6, area.height).max(1);
    let st = DialogStyle::from_colors(cs);
    let buttons = [DialogButton { id: ButtonId::Close, label: "Close" }];
    // The zoom label + key hint ride the title line (SQ-0486) — images only; a
    // sound preview has no zoom to show.
    let title = if pv.image.is_some() {
        format!("{}  ({} · +/- zoom · 0 fit)", pv.title, pv.zoom.label())
    } else {
        pv.title.clone()
    };
    let spec = DialogSpec {
        title: &title,
        placement: Placement::Centered { w, h },
        buttons: &buttons,
        show_close: true,
        default: Some(ButtonId::Close),
        focus: None,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);

    let content = rects.content;
    // Render the image fitted (or zoomed) + centred, else the status line centred.
    let mut drew_image = false;
    if let (Some(picker), Some(img)) = (picker, pv.image.as_ref()) {
        if content.width >= 1 && content.height >= 1 {
            let fresh = matches!(&pv.proto, Some((w, h, z, _, _))
                if *w == content.width && *h == content.height && *z == pv.zoom);
            if !fresh {
                let target = ratatui::layout::Size::new(content.width, content.height);
                let built = match pv.zoom {
                    PreviewZoom::Fit => {
                        // The fitted view is a reduction — a blorb Pict is usually
                        // larger than the modal — so it needs the area filter, and
                        // a cut-out picture needs its alpha associated (SQ-0829).
                        // On half-blocks it is ONE reduction, straight onto the
                        // sample grid the backend draws (SQ-0979). The zoom arm
                        // below is already right by construction: an integer
                        // magnification is what Nearest is FOR.
                        app::render::graphics::fitted_protocol(picker, img, target, false)
                    }
                    PreviewZoom::Factor(n) => {
                        // Scale to an exact integer multiple of the native pixel
                        // size (nearest-neighbour, so pixel art stays crisp),
                        // then centre-crop to the available pixel budget rather
                        // than letting `Resize::Fit` shrink it back down.
                        let font = picker.font_size();
                        let budget = (
                            content.width as u32 * font.width as u32,
                            content.height as u32 * font.height as u32,
                        );
                        let scaled_w = img.width().saturating_mul(n).max(1);
                        let scaled_h = img.height().saturating_mul(n).max(1);
                        let scaled = img.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Nearest);
                        let (cx, cy, cw, ch) = center_crop_rect((scaled_w, scaled_h), budget);
                        let cropped = scaled.crop_imm(cx, cy, cw, ch);
                        picker.new_protocol(cropped, target, ratatui_image::Resize::Fit(None)).ok()
                    }
                };
                if let Some(built) = built {
                    // Freed only once the rebuild actually produced something — a
                    // failed build (an unlikely encode error) must leave the
                    // surviving proto, and its terminal upload, alone (SQ-1190).
                    let old = pv.proto.take();
                    cover.queue_external_delete(old.and_then(|t| t.4));
                    pv.proto = Some((content.width, content.height, pv.zoom, built, None));
                }
            }
            let mut placed_id = None;
            if let Some((_, _, _, proto, _)) = &pv.proto {
                let sz = proto.size();
                let uw = sz.width.min(content.width);
                let uh = sz.height.min(content.height);
                let dest = Rect::new(
                    content.x + (content.width - uw) / 2,
                    content.y + (content.height - uh) / 2,
                    uw,
                    uh,
                );
                placed_id = Some(app::render::graphics::place_protocol(proto, dest, buf));
                drew_image = true;
            }
            if let Some(id) = placed_id {
                if let Some(entry) = pv.proto.as_mut() {
                    entry.4 = id;
                }
            }
        }
    }
    if !drew_image {
        if let Some(status) = &pv.status {
            let text = truncate_to_width(status, content.width as usize);
            let tx = content.x + (content.width.saturating_sub(UnicodeWidthStr::width(text.as_str()) as u16)) / 2;
            let ty = content.y + content.height / 2;
            draw_str_clipped(buf, tx, ty, &text, cs.theme.get("story_info_value").style, content);
        }
    }
    rects
}

/// Translate a raw Blorb resource usage FourCC into a human-readable label.
fn resource_usage_label(usage: &str) -> String {
    match usage.trim() {
        "Exec" => "Code".into(),
        "Pict" => "Image".into(),
        "Snd" => "Sound".into(),
        "Data" => "Data".into(),
        other => other.to_string(), // unknown: show raw (trimmed), nothing hidden
    }
}

/// Translate a raw Blorb chunk-type FourCC into a human-readable label.
fn resource_type_label(chunk_type: &str) -> String {
    match chunk_type.trim() {
        "ZCOD" => "Z-code".into(),
        "GLUL" => "Glulx".into(),
        "FORM" => "AIFF".into(),
        "OGGV" => "Ogg Vorbis".into(),
        "MOD" => "MOD".into(),
        "PNG" => "PNG".into(),
        "JPEG" => "JPEG".into(),
        "GIF" => "GIF".into(),
        other => other.to_string(), // unknown: raw FourCC
    }
}

/// Format a byte count as `"N B"` / `"N KB"` / `"N.N MB"`.
fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// Present-only feature badge words, folding in aux-derived signals (an
/// associated blorb's sound/picture chunks, or a resolved hint index).
fn feature_words(f: &app::picker::Features, aux: Option<&app::picker::StoryAux>) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut sound = f.sound;
    let mut graphics = f.graphics;
    if let Some((_, chunks)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
        if chunks.iter().any(|c| c.usage == "Snd ") {
            sound = true;
        }
        if chunks.iter().any(|c| c.usage == "Pict") {
            graphics = true;
        }
    }
    if sound {
        v.push("sound");
    }
    if graphics {
        v.push("graphics");
    }
    if f.colour == Some(true) {
        v.push("colour");
    }
    if f.hints || aux.map(|a| a.hints_available).unwrap_or(false) {
        v.push("hints");
    }
    v
}

/// Selection + status line after an IFDB download's rescan (SQ-0659).
///
/// `found` is the downloaded file's position in the rescanned list; `previous`
/// is the pre-download selection's position in that same rescanned list. A
/// miss — the file was downloaded but didn't survive the rescan as a
/// launchable story — must NOT silently land the cursor on row 0 under a
/// success toast: keep the user's previous selection and say what happened.
fn ifdb_download_landing(
    found: Option<usize>,
    previous: Option<usize>,
    len: usize,
    name: &str,
) -> (usize, String) {
    match found {
        Some(idx) => (idx, format!("Downloaded {name}")),
        None => (
            previous.unwrap_or(0).min(len.saturating_sub(1)),
            format!("Downloaded {name}, but it did not scan as a playable story"),
        ),
    }
}

#[cfg(test)]
mod tests {
    /// The shipped browser keymap, which is what the footer hints are drawn
    /// from (SQ-0796). Every draw test uses it, since none of them is about a
    /// user's rebinding.
    fn km() -> app::keymap::KeyMap {
        app::keymap::KeyMap::default()
    }

    /// A wheel notch goes to the topmost open surface, and no further. The
    /// launch-options dialog (SQ-0789) had no wheel handling at all, so a notch
    /// over the open dialog scrolled the story list BEHIND it — the list you can
    /// see moving around under a modal you are talking to. Its own option list is
    /// shorter than its dialog, so there is nothing there to scroll under
    /// SQ-0831's rule; the fix is that the modal eats the notch (SQ-0832).
    #[test]
    fn the_launch_options_dialog_swallows_the_wheel_instead_of_leaking_it_to_the_list() {
        use super::{wheel_target, WheelTarget};

        // Nothing open: the notch is the story list's.
        assert_eq!(wheel_target(false, false, false, false, false, false), WheelTarget::StoryList);
        assert_eq!(wheel_target(false, false, false, false, false, true), WheelTarget::InfoPanel);

        // The launch dialog is topmost — over the info panel, and over every
        // other modal it can be opened on top of.
        assert_eq!(wheel_target(true, false, false, false, false, false), WheelTarget::Swallowed);
        assert_eq!(wheel_target(true, false, false, false, false, true), WheelTarget::Swallowed);
        assert_eq!(wheel_target(true, false, false, true, true, true), WheelTarget::Swallowed);

        // SQ-1227: the key reference and the per-story menu swallow it too —
        // neither scrolls, and neither may let the list slide behind it.
        assert_eq!(wheel_target(false, true, false, false, false, true), WheelTarget::Swallowed);
        assert_eq!(wheel_target(false, false, true, false, false, true), WheelTarget::Swallowed);

        // The rest of the ladder is unchanged (SQ-0831/SQ-0486).
        assert_eq!(wheel_target(false, false, false, true, false, true), WheelTarget::Search);
        assert_eq!(wheel_target(false, false, false, false, true, true), WheelTarget::PreviewZoom);
    }


    /// **The anti-drift guard (SQ-0796).** The browser's key dispatch must stay
    /// keyed on `BrowserAction`, which only a `slash::COMMANDS` entry can produce
    /// — so the region may not inspect the keystroke at all. Add a hardcoded
    /// `k.code` arm for a new gesture and this fails; the only way to make a key
    /// do something in the browser is to put it in the registry.
    ///
    /// Read off the source because that is where the property lives: no runtime
    /// assertion can tell you a match arm you did not write is absent.
    #[test]
    fn browser_dispatch_never_reads_the_key_event() {
        const BEGIN: &str = "── BROWSER KEY DISPATCH";
        const END: &str = "── END BROWSER KEY DISPATCH";
        let src = include_str!("picker_ui.rs");
        let start = src.find(BEGIN).expect("the dispatch region's opening marker");
        let end = start + src[start..].find(END).expect("the dispatch region's closing marker");
        let region = &src[start..end];
        assert!(region.len() > 500, "the region markers must bracket the real dispatch");
        assert!(
            region.contains("action_for_key"),
            "the region must be the registry-driven dispatch"
        );
        for banned in ["k.code", "k.modifiers", "KeyCode", "KeyModifiers", "KeyEvent", "shift"] {
            assert!(
                !region.contains(banned),
                "the browser dispatch must not mention `{banned}` — a gesture that \
                 reads the keystroke here bypasses the slash::COMMANDS registry \
                 (SQ-0796). Add a Context::Browser command instead."
            );
        }
    }

    // ── Cell-size refresh (SQ-0988/SQ-0992) ───────────────────────────────────

    /// A cell-size refresh moves the cell and leaves the rest of the picker
    /// alone — the protocol it was queried for, and the capability list behind
    /// it (SQ-0992).
    ///
    /// The capability half of this assertion is weaker here than it looks:
    /// `Picker`'s fields are private and there is no way to build one carrying
    /// capabilities from outside the crate, so the list this compares is empty.
    /// The seeded version of the same property lives where the fields are
    /// reachable, in `ratatui-image`'s own
    /// `picker::tests::test_set_font_size_keeps_the_rest_of_the_picker`. What
    /// this case pins is the arithmetic, and its neighbour below pins the shape
    /// that made capabilities survivable at all.
    #[test]
    fn a_cell_size_refresh_moves_the_cell_and_nothing_else() {
        use ratatui_image::picker::ProtocolType;
        use ratatui_image::FontSize;

        let mut picker = ratatui_image::picker::Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        let capabilities_before = picker.capabilities().clone();
        let was = picker.font_size();

        // The same measurement is not a change, and the caller is told so — it
        // throws away everything it fitted against the old cell on a `true`.
        assert!(!super::apply_cell_size(&mut picker, FontSize::new(was.width, was.height)));
        assert_eq!((was.width, was.height), (picker.font_size().width, picker.font_size().height));

        // A different cell: the size moves, and nothing else does.
        assert!(super::apply_cell_size(&mut picker, FontSize::new(7, 15)));
        assert_eq!((7, 15), (picker.font_size().width, picker.font_size().height));
        assert_eq!(ProtocolType::Kitty, picker.protocol_type());
        assert_eq!(&capabilities_before, picker.capabilities());
    }

    /// **The anti-drift guard (SQ-0992).** The refresh must MUTATE the picker.
    /// Rebuilding it preserves exactly the fields whoever wrote the rebuild
    /// remembered to copy across, and the one that was forgotten —
    /// `capabilities`, which `Picker::from_fontsize` constructs empty — costs a
    /// kitty session its `o=z` compression the moment the user changes font
    /// size, silently and until relaunch.
    ///
    /// Read off the source because that is where the property lives: with no way
    /// to build a picker that carries capabilities from outside the crate, no
    /// runtime assertion in this crate can tell a rebuild from a mutation.
    #[test]
    fn a_cell_size_refresh_never_rebuilds_the_picker() {
        let src = include_str!("picker_ui.rs");
        let start = src.find("pub(crate) fn refresh_cell_size(picker").expect("the refresh");
        let tail = start + src[start..].find("fn apply_cell_size(").expect("the applier");
        let end = tail + src[tail..].find("\n}\n").expect("the applier's closing brace");
        let region = &src[start..end];

        assert!(region.len() > 300, "the bounds must bracket both function bodies");
        assert!(region.contains("set_font_size"), "the refresh must go through the setter");

        for banned in ["from_fontsize", "Picker::from", "*picker ="] {
            assert!(
                !region.contains(banned),
                "the cell-size refresh must not mention `{banned}` — building a \
                 replacement picker drops every capability the original was \
                 queried for, `KittyCompression` among them (SQ-0992). Mutate the \
                 picker instead."
            );
        }
    }

    // ── Story-picker row badges (type + present artifacts) ─────────────────────

    /// SQ-0659: where the cursor lands after an IFDB download's rescan.
    #[test]
    fn ifdb_download_landing_selects_the_new_story_or_keeps_the_previous_one() {
        // Hit: select the downloaded story, toast success.
        let (idx, line) = super::ifdb_download_landing(Some(3), Some(7), 10, "game.z5");
        assert_eq!(idx, 3);
        assert_eq!(line, "Downloaded game.z5");

        // Miss: keep the previous selection — never a silent jump to row 0 —
        // and the status line must NOT read as a plain success.
        let (idx, line) = super::ifdb_download_landing(None, Some(7), 10, "game.z5");
        assert_eq!(idx, 7, "previous selection is kept on a miss");
        assert_ne!(line, "Downloaded game.z5", "a miss must not toast plain success");
        assert!(line.contains("but"), "the miss is called out: {line}");

        // Miss with the previous selection gone too: clamp into the new list.
        assert_eq!(super::ifdb_download_landing(None, None, 4, "x").0, 0);
        assert_eq!(super::ifdb_download_landing(None, Some(9), 3, "x").0, 2);
        // Empty list: index 0 without panicking (rendering guards handle len 0).
        assert_eq!(super::ifdb_download_landing(None, None, 0, "x").0, 0);
    }

    #[test]
    fn interp_label_formats_type_version_and_blorb() {
        use app::picker::{Engine, Features, StoryMeta};
        let meta = |engine: Engine, version: Option<&str>| StoryMeta {
            size_bytes: 0, story_bytes: 0, modified: None, engine, format: String::new(),
            version: version.map(String::from), serial: None, release: None, ifid: String::new(),
            features: Features::default(), self_blorb: None, disk_image: None, disk_entry: None,
            author: None, year: None,
            genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        // Z-code: "Z<v>", plus " (blorb)" only when blorb'd.
        assert_eq!(super::interp_label(&meta(Engine::ZCode, Some("5")), false), "Z5");
        assert_eq!(super::interp_label(&meta(Engine::ZCode, Some("3")), true), "Z3 (blorb)");
        assert_eq!(super::interp_label(&meta(Engine::ZCode, None), false), "Z");
        // Glulx: "G<v>", never a blorb suffix (Glulx is effectively always blorbed).
        assert_eq!(super::interp_label(&meta(Engine::Glulx, Some("3.1.2")), true), "G3.1.2");
        assert_eq!(super::interp_label(&meta(Engine::Glulx, None), false), "Glulx");
        // Scott: "Scott", plus " (blorb)" for the graphic .blb versions.
        assert_eq!(super::interp_label(&meta(Engine::Scott, None), false), "Scott");
        assert_eq!(super::interp_label(&meta(Engine::Scott, None), true), "Scott (blorb)");
        // Widest label ("Scott (blorb)") fits the column.
        assert!(super::interp_label(&meta(Engine::Scott, None), true).len() <= super::INTERP_COL_W as usize);
        assert!(super::interp_label(&meta(Engine::ZCode, Some("8")), true).len() <= super::INTERP_COL_W as usize);
    }

    /// SQ-0737: a story mounted off a release floppy names that container in the
    /// same slot the blorb suffix uses, so the disk image is not shown as a bare
    /// story file — and SQ-0837: it names WHICH container, so a Macintosh disk
    /// is not labelled as an Amiga one.
    #[test]
    fn interp_label_names_the_disk_image_container() {
        use app::hints::DiskImage;
        use app::picker::{Engine, Features, StoryMeta};
        let meta = |disk_image: Option<DiskImage>| StoryMeta {
            size_bytes: 0, story_bytes: 0, modified: None, engine: Engine::ZCode, format: String::new(),
            version: Some("6".into()), serial: None, release: None, ifid: String::new(),
            features: Features::default(), self_blorb: None, disk_image, disk_entry: None,
            author: None, year: None,
            genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        assert_eq!(super::interp_label(&meta(Some(DiskImage::Adf)), false), "Z6 (ADF)");
        assert_eq!(super::interp_label(&meta(Some(DiskImage::Hfs)), false), "Z6 (HFS)");
        // …and SQ-0833/SQ-0835: the PC and the Atari ST, which share a
        // filesystem and must still be named apart, because they are different
        // machines and the column is the only place a player is told which.
        assert_eq!(super::interp_label(&meta(Some(DiskImage::Fat12Dos)), false), "Z6 (DOS)");
        assert_eq!(super::interp_label(&meta(Some(DiskImage::Fat12AtariSt)), false), "Z6 (ST)");
        // Not a disk image: exactly what it rendered before.
        assert_eq!(super::interp_label(&meta(None), false), "Z6");
        assert_eq!(super::interp_label(&meta(None), true), "Z6 (blorb)");
        // Every format, so a new one cannot arrive with a label that overflows
        // the column — the enumeration is the table's, not a copy of it.
        for image in DiskImage::all() {
            assert!(
                super::interp_label(&meta(Some(image)), false).len() <= super::INTERP_COL_W as usize
            );
        }
    }

    /// End to end on real media (skips vacuously — `stories/` is gitignored):
    /// every release floppy in the story directory resolves through the picker's
    /// own scan and lands in the TYPE column as `Z<v> (ADF)` off an Amiga disk or
    /// `Z<v> (HFS)` off a Macintosh one, while a `.z*` file beside it keeps a
    /// plain `Z<v>`. The container is identified by the disk's own filesystem
    /// during the mount, not by its extension.
    #[test]
    fn a_real_disk_image_lists_with_its_container() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // no story media here — skip
        };
        let data_base = std::env::temp_dir().join(format!("lanthorn-adf-label-{}", std::process::id()));
        let mut saw_image = false;
        let mut saw_bare = false;
        for e in entries.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            let container = match ext.as_str() {
                "adf" => Some("ADF"),
                "image" => Some("HFS"),
                "z5" | "z6" => None,
                _ => continue,
            };
            let Some(entry) = app::picker::resolve_entry(&path, &data_base) else {
                continue; // not launchable — the picker wouldn't list it either
            };
            let label = super::interp_label(&entry.meta, false);
            let v = entry.meta.version.clone().unwrap_or_default();
            match container {
                Some(c) => {
                    saw_image = true;
                    assert_eq!(
                        entry.meta.disk_image.map(|d| d.label()),
                        Some(c),
                        "{} mounted off a {c} disk image",
                        path.display()
                    );
                    assert_eq!(label, format!("Z{v} ({c})"), "{}", path.display());
                }
                None => {
                    saw_bare = true;
                    assert!(entry.meta.disk_image.is_none(), "{} is a plain story file", path.display());
                    assert!(!label.contains('('), "{} untouched: {label:?}", path.display());
                }
            }
        }
        let _ = std::fs::remove_dir_all(&data_base);
        if saw_image {
            assert!(saw_bare, "the bare-story half of the comparison needs a .z5/.z6 present");
        }
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16, area: ratatui::layout::Rect) -> String {
        (area.left()..area.right())
            .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
            .collect()
    }

    /// `needle`'s CHAR (column) index within `row_text`'s output, not its byte
    /// index — a preceding multi-byte cell (e.g. the "▸" selection marker)
    /// would otherwise overcount a plain `.find()`.
    fn char_pos(row: &str, needle: &str) -> usize {
        let byte_idx = row.find(needle).unwrap_or_else(|| panic!("{needle:?} not found in {row:?}"));
        row[..byte_idx].chars().count()
    }

    fn make_two_test_stories() -> Vec<app::picker::StoryEntry> {
        use app::picker::{Engine, Features, StoryEntry, StoryMeta};
        let mk = |title: &str, engine: Engine| StoryEntry {
            path: std::path::PathBuf::from(format!("/tmp/{title}.z5")),
            title: title.into(),
            filename: format!("{title}.z5"),
            meta: StoryMeta {
                size_bytes: 1, story_bytes: 1, modified: None, engine, format: "Z-code".into(),
                version: None, serial: None, release: None, ifid: title.into(),
                features: Features::default(), self_blorb: None, disk_image: None, disk_entry: None,
                author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
            },
            hint_sidecar: None,
            kind: app::picker::RowKind::Story,
        };
        vec![mk("Zork", Engine::ZCode), mk("Anchorhead", Engine::Glulx)]
    }

    /// Build a story entry with an explicit author/year (or none), for the
    /// column-layout tests below.
    fn story_with_meta(title: &str, author: Option<&str>, year: Option<&str>) -> app::picker::StoryEntry {
        use app::picker::{Engine, Features, StoryEntry, StoryMeta};
        StoryEntry {
            path: std::path::PathBuf::from(format!("/tmp/{title}.z5")),
            title: title.into(),
            filename: format!("{title}.z5"),
            meta: StoryMeta {
                size_bytes: 1, story_bytes: 1, modified: None, engine: Engine::ZCode, format: "Z-code".into(),
                version: None, serial: None, release: None, ifid: title.into(),
                features: Features::default(), self_blorb: None, disk_image: None, disk_entry: None,
                author: author.map(String::from), year: year.map(String::from),
                genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
            },
            hint_sidecar: None,
            kind: app::picker::RowKind::Story,
        }
    }

    /// A folder row is its label and `folder`, nothing else: no
    /// "(no metadata yet)" in the author column, and it sits above the stories
    /// whatever the sort says. The header counts folders apart from stories.
    #[test]
    fn folder_rows_paint_their_label_and_nothing_else() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let mut stories = make_two_test_stories();
        stories.push(app::picker::StoryEntry::folder(std::path::PathBuf::from("/tmp/zcode"), "zcode/"));
        stories.push(app::picker::StoryEntry::folder(std::path::PathBuf::from("/"), app::picker::PARENT_LABEL));
        app::picker::sort_stories(&mut stories, app::picker::Sort::default());
        let list = app::list_scroll::ListScroll::new();
        let badges = vec![app::picker::RowBadges::default(); stories.len()];
        let sym = app::style::finalize_symbols(&app::style::load_style(None, std::path::Path::new("/nonexistent")).0.symbols);
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 120, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let header = row_text(&buf, 0, area);
        assert!(header.contains("2 found, 1 folder in /tmp"), "header counts stories and folders apart: {header:?}");
        let r2 = row_text(&buf, 2, area);
        let r3 = row_text(&buf, 3, area);
        assert!(r2.contains(".."), "the way up is the first row: {r2:?}");
        assert!(r3.contains("zcode/") && r3.contains("folder"), "then the folder, typed `folder`: {r3:?}");
        assert!(!r2.contains("no metadata") && !r3.contains("no metadata"), "a folder has no metadata to be missing");
        assert!(row_text(&buf, 4, area).contains("Anchorhead"), "stories follow the folders");
    }

    /// The gallery's header says it is showing the folder and everything
    /// under it, and how far the index has got while it is still building.
    #[test]
    fn the_gallery_heading_names_the_recursive_scope() {
        let root = std::path::Path::new("/tmp/lib");
        let stories = make_two_test_stories();
        let sub = root.join("zcode");
        let building = super::PickerHeading {
            dir: &sub,
            root,
            find: None,
            all_folders: Some(super::IndexStatus { indexed: 2, done: false }),
        };
        let line = building.line(&stories, "g: list");
        // Built from `display()`, since the separator is the platform's.
        let expected = format!("2 in {} and its folders · indexing, 2 so far", sub.display());
        assert!(line.contains(&expected), "{line:?}");
        let done = super::PickerHeading { all_folders: Some(super::IndexStatus { indexed: 2, done: true }), ..building };
        let line = done.line(&stories, "g: list");
        assert!(line.contains("and its folders)") && !line.contains("indexing"), "{line:?}");
    }

    /// While finding, a match shows the folder it came from after its title;
    /// in a plain folder view, where every row is in the header's folder, it
    /// shows nothing of the kind.
    #[test]
    fn find_matches_carry_their_folder_and_folder_views_do_not() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let mut stories = make_two_test_stories();
        stories[0].path = std::path::PathBuf::from("/tmp/zcode/german/Zork.z5");
        let list = app::list_scroll::ListScroll::new();
        let badges = vec![app::picker::RowBadges::default(); stories.len()];
        let sym = app::style::finalize_symbols(&app::style::load_style(None, std::path::Path::new("/nonexistent")).0.symbols);
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 120, 8);
        let root = std::path::Path::new("/tmp");

        let mut buf = Buffer::empty(area);
        let finding = super::PickerHeading {
            dir: root,
            root,
            find: Some(super::FindStatus { query: "zor", indexed: 2, done: false }),
            all_folders: None,
        };
        super::draw_story_picker(&stories, &list, &badges, &glyphs, &finding, &cs, &km(), app::picker::Sort::default(), area, &mut buf);
        let header = row_text(&buf, 0, area);
        assert!(header.contains("2 matches for “zor” in /tmp") && header.contains("indexing, 2 so far"), "{header:?}");
        let rows = (2..4).map(|y| row_text(&buf, y, area)).collect::<Vec<_>>().join("\n");
        assert!(rows.contains("Zork  zcode/german/"), "the nested match names its folder: {rows:?}");
        let anchorhead = rows.lines().find(|l| l.contains("Anchorhead")).unwrap_or_default();
        assert!(!anchorhead.contains('/'), "a match at the root wears no label: {anchorhead:?}");

        // A folder view lists the folder's own stories, so relative to it there
        // is nothing to say (and a row from outside the folder says nothing
        // either).
        let mut buf = Buffer::empty(area);
        let german = root.join("zcode/german");
        super::draw_story_picker(&stories, &list, &badges, &glyphs, &super::PickerHeading::browse(&german), &cs, &km(), app::picker::Sort::default(), area, &mut buf);
        let rows = (2..4).map(|y| row_text(&buf, y, area)).collect::<Vec<_>>().join("\n");
        assert!(rows.contains("Zork") && !rows.contains('/'), "a folder view labels nothing: {rows:?}");
    }

    #[test]
    fn row_renders_type_badge_and_present_artifacts() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        // One Z-code story with all three artifacts, one Glulx story with only a save.
        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: true, hint: app::picker::HintBadge::Present },
            app::picker::RowBadges { blorb: false, save: true, hint: app::picker::HintBadge::None },
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());

        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        let dir = std::path::Path::new("/tmp");
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(dir), &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );

        let row0 = row_text(&buf, 2, area); // list starts at area.y + 2
        let row1 = row_text(&buf, 3, area);
        // Type AND blorb moved into the TYPE column (SQ-0369), so the badge
        // cluster is just [save][hint], adjacent and no separators.
        assert!(row0.contains("SH"), "save+hint adjacent, no type/blorb glyph: {row0:?}");
        assert!(row1.contains("S"), "got: {row1:?}");
        assert!(!row1.contains("H"), "absent hint omitted: {row1:?}");
        // The blorb'd Z-code story shows "(blorb)" in its TYPE column; the Glulx
        // story shows its interpreter label. Neither shows a B badge.
        assert!(row0.contains("(blorb)"), "blorb'd Z story tagged in TYPE column: {row0:?}");
        assert!(row1.contains("Glulx"), "Glulx story shows its type label: {row1:?}");

        // Fixed-slot alignment: the save glyph must land at the same column
        // in both rows regardless of which other artifacts are present.
        // (char index, not byte index — row0's "▸ " marker is multi-byte.)
        let save_x0 = row0.chars().position(|c| c == 'S').expect("row0 has save glyph");
        let save_x1 = row1.chars().position(|c| c == 'S').expect("row1 has save glyph");
        assert_eq!(save_x0, save_x1, "save glyph column must be fixed across rows");
    }

    /// An `Available` hint renders the lowercase glyph, distinct from a present
    /// hint's uppercase `H`.
    #[test]
    fn row_renders_lowercase_glyph_for_available_hint() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: false, hint: app::picker::HintBadge::Available },
            app::picker::RowBadges::default(),
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(&stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
                          &cs, &km(), app::picker::Sort::default(), area, &mut buf);
        let row0 = row_text(&buf, 2, area);
        assert!(row0.contains('h'), "available hint shows lowercase glyph: {row0:?}");
        assert!(!row0.contains('H'), "available hint is NOT the uppercase present glyph: {row0:?}");
    }

    #[test]
    fn row_uses_configured_badge_glyphs() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut sym = app::config::SymbolConfig::default();
        sym.badge_save = "§".into();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: true, hint: app::picker::HintBadge::None },
            app::picker::RowBadges::default(),
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(&stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
                          &cs, &km(), app::picker::Sort::default(), area, &mut buf);
        let row0 = row_text(&buf, 2, area);
        // The configured save glyph is used for the artifact badge. Type and
        // blorb are not badges at all — SQ-0369 made them the TYPE column's
        // text, and SQ-1160 retired the glyph keys that were still themeable
        // for a mark nothing drew. What is left to assert is the column.
        assert!(row0.contains('§'), "configured save glyph used: {row0:?}");
        assert!(row0.contains("(blorb)"), "blorb is a TYPE suffix, not a badge: {row0:?}");
    }

    // ── Story-picker list: columns, header, sort ────────────────────────────────

    #[test]
    fn header_row_shows_columns_and_active_direction_arrow() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("Curses", Some("Graham Nelson"), Some("1993")),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);

        // Default sort (Title, ascending): only TITLE carries an arrow.
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let header = row_text(&buf, 1, area); // header row is area.y + 1
        assert!(header.contains("TITLE ▲"), "active column shows the ascending arrow: {header:?}");
        assert!(header.contains("AUTHOR"), "author header present: {header:?}");
        assert!(!header.contains("AUTHOR ▲") && !header.contains("AUTHOR ▼"), "inactive column has no arrow: {header:?}");
        assert!(header.contains("YEAR"), "year header present: {header:?}");
        assert!(!header.contains("YEAR ▲") && !header.contains("YEAR ▼"), "inactive column has no arrow: {header:?}");

        // Sort by Year, descending: only YEAR carries the down arrow.
        let mut buf2 = Buffer::empty(area);
        let sort2 = app::picker::Sort { key: app::picker::SortKey::Year, desc: true };
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), sort2, area, &mut buf2,
        );
        let header2 = row_text(&buf2, 1, area);
        assert!(header2.contains("YEAR ▼"), "active column shows the descending arrow: {header2:?}");
        assert!(!header2.contains("TITLE ▲") && !header2.contains("TITLE ▼"), "{header2:?}");
    }

    #[test]
    fn row_renders_author_and_year_aligned_across_rows() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("Curses", Some("Graham Nelson"), Some("1993")),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let row0 = row_text(&buf, 2, area);
        let row1 = row_text(&buf, 3, area);
        assert!(row0.contains("Michael S. Gentry"), "{row0:?}");
        assert!(row0.contains("1998"), "{row0:?}");
        assert!(row1.contains("Graham Nelson"), "{row1:?}");
        assert!(row1.contains("1993"), "{row1:?}");

        let author_x0 = char_pos(&row0, "Michael");
        let author_x1 = char_pos(&row1, "Graham");
        assert_eq!(author_x0, author_x1, "author column must align across rows");
        let year_x0 = char_pos(&row0, "1998");
        let year_x1 = char_pos(&row1, "1993");
        assert_eq!(year_x0, year_x1, "year column must align across rows");
    }

    #[test]
    fn row_with_no_author_shows_no_metadata_placeholder_styled_correctly() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        // A fresh bare-z file with no fetched/embedded metadata — the common
        // case for a library nobody has run a fetch on yet, not an edge case.
        // A second, unrelated story keeps the no-metadata row UNSELECTED
        // (selection highlight intentionally overrides column colors, same
        // as the badge cluster does — so this checks the plain-row style).
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("zork2-r63-s860811", None, None),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(2);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let row1 = row_text(&buf, 3, area);
        assert!(row1.contains("(no metadata yet)"), "reads as 'nothing fetched yet': {row1:?}");

        // Styled via theme.get("story_no_metadata"), not "story_author" —
        // terminal_default gives them distinct fg colors (DarkGray vs White), so
        // this checks the right selector was actually applied, not just that
        // text is present.
        let no_metadata_fg = cs.theme.get("story_no_metadata").style.fg;
        let author_fg = cs.theme.get("story_author").style.fg;
        let x = char_pos(&row1, "(no metadata yet)") as u16;
        let cell = buf.cell((area.left() + x, 3)).unwrap();
        assert_eq!(cell.fg, no_metadata_fg.unwrap(), "placeholder must use story_no_metadata's color");
        assert_ne!(no_metadata_fg, author_fg, "sanity: the two styles must actually differ");
    }

    #[test]
    fn columns_drop_year_then_author_as_width_narrows() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);

        // (width, author shown, year shown). Right zone = INTERP_COL_W(13) +
        // COL_GAP(2) + cluster_w(save+hint=2) = 17, reserved 18; so avail =
        // width - 20. year needs avail >= 38 (width >= 58); author needs avail
        // >= 30 (width >= 50). Below that: title + right-zone only.
        for &(width, want_author, want_year) in &[
            (70u16, true, true),
            (58, true, true),
            (57, true, false),
            (50, true, false),
            (49, false, false),
            (30, false, false),
        ] {
            let area = Rect::new(0, 0, width, 10);
            let mut buf = Buffer::empty(area);
            super::draw_story_picker(
                &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
                &cs, &km(), app::picker::Sort::default(), area, &mut buf,
            );
            let row = row_text(&buf, 2, area);
            assert_eq!(row.contains("Michael S. Gentry"), want_author, "width {width}: {row:?}");
            assert_eq!(row.contains("1998"), want_year, "width {width}: {row:?}");

            // The TYPE column stays right-aligned at a fixed offset regardless of
            // which text columns show — proving no gap opened in front of the
            // right-hand zone as columns drop. This story is ZCode/no-version/
            // no-blorb, so its interpreter label is "Z".
            let interp_x = width - 1 - 2 /*cluster*/ - super::COL_GAP - super::INTERP_COL_W;
            let cell = buf.cell((interp_x, 2)).unwrap();
            assert_eq!(cell.symbol(), "Z", "TYPE column at col {interp_x} for width {width}: {row:?}");
        }
    }

    #[test]
    fn long_author_truncates_with_ellipsis_within_column() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let long_author = "Marc Blank and Dave Lebling and a Whole Lot More People";
        let stories = vec![story_with_meta("Zork I", Some(long_author), Some("1980"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let row0 = row_text(&buf, 2, area);
        assert!(!row0.contains(long_author), "long author must be truncated: {row0:?}");
        assert!(row0.contains('…'), "truncated author ends with an ellipsis: {row0:?}");
        assert!(row0.contains("1980"), "year column unaffected by the author overrun: {row0:?}");
        // TYPE column ("Z" here) stays put at its fixed right-zone offset.
        let interp_x = 60u16 - 1 - 2 - super::COL_GAP - super::INTERP_COL_W;
        assert_eq!(
            buf.cell((interp_x, 2)).unwrap().symbol(), "Z",
            "TYPE column unaffected by the author overrun"
        );
    }

    #[test]
    fn author_column_grows_to_show_a_longer_name_when_there_is_room() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        // 27 wide — past the old fixed 20-col author column, but under the cap.
        let author = "Brian Moriarty (Infocom)  X";
        let stories = vec![story_with_meta("Trinity", Some(author), Some("1986"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        // A wide terminal: title's minimum is easily met, so the author column
        // should grow to show the whole name rather than truncate at 20.
        let area = Rect::new(0, 0, 100, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let row = row_text(&buf, 2, area);
        assert!(row.contains(author), "author shown in full when there is room: {row:?}");
        assert!(!row.contains('…'), "no ellipsis when the column grew to fit: {row:?}");
    }

    #[test]
    fn header_rects_line_up_with_drawn_header_text() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        let (_, _, header_rects) = super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        // 60 cells is too narrow for the RATING column, so four headers show.
        assert_eq!(header_rects.len(), 4, "title/author/year/type at this width: {header_rects:?}");
        for (key, rect) in &header_rects {
            let expected_char = match key {
                app::picker::SortKey::Title => "T",
                app::picker::SortKey::Author => "A",
                app::picker::SortKey::Year => "Y",
                app::picker::SortKey::Rating => "R", // "RATING"
                app::picker::SortKey::Type => "T",   // "TYPE"
            };
            let cell = buf.cell((rect.x, rect.y)).unwrap();
            assert_eq!(
                cell.symbol(), expected_char,
                "{key:?} rect at ({}, {}) must start where its header text is actually drawn",
                rect.x, rect.y
            );
        }
    }

    /// SQ-0529. The RATING column shows IFDB's average to one decimal followed by
    /// the number of votes it is over — `3.8 (226)` — and an
    /// unrated story (or one never fetched — the sidecar only gained the field
    /// at `FETCH_VERSION` 2, so `r` repopulates it) leaves the cell EMPTY. The
    /// cell is located via the returned header rect rather than a hard-coded
    /// column, so the assertion survives a layout tweak.
    #[test]
    fn rating_column_shows_one_decimal_and_leaves_unrated_rows_blank() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        let mut rated = story_with_meta("Zork", Some("Marc Blank"), Some("1980"));
        rated.meta.ifdb_rating = Some(3.818_584); // the fixture's real average
        rated.meta.ifdb_rating_count = Some(226);
        let stories = vec![rated, story_with_meta("Nobody", Some("A N Other"), Some("1999"))];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());

        // Wide enough for every column (the RATING column is the first to drop).
        let area = Rect::new(0, 0, 100, 10);
        let mut buf = Buffer::empty(area);
        let (_, _, header_rects) = super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), app::picker::Sort::default(), area, &mut buf,
        );
        let rect = header_rects
            .iter()
            .find(|(k, _)| *k == app::picker::SortKey::Rating)
            .map(|(_, r)| *r)
            .expect("the RATE column is shown at 100 cells");

        let cell = |row_y: u16| -> String {
            row_text(&buf, row_y, area)
                .chars()
                .skip(rect.x as usize)
                .take(rect.width as usize)
                .collect()
        };
        assert_eq!(cell(1).trim(), "RATING", "the column header sits over its own rect");
        assert_eq!(
            cell(2).trim(), "3.8 (226)",
            "one decimal plus the vote count — not stars, not the raw 3.818584"
        );
        assert_eq!(
            cell(3).trim(), "",
            "no rating renders as blank; a 0.0 would read as a real, damning score"
        );
    }

    /// The RATING column joins the sortable set: it takes the direction arrow
    /// when active, exactly like TITLE/AUTHOR/YEAR/TYPE.
    #[test]
    fn rating_header_takes_the_sort_arrow_when_active() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![story_with_meta("Zork", Some("Marc Blank"), Some("1980"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let area = Rect::new(0, 0, 100, 10);

        let mut buf = Buffer::empty(area);
        let sort = app::picker::Sort { key: app::picker::SortKey::Rating, desc: true };
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, &super::PickerHeading::browse(std::path::Path::new("/tmp")), &cs, &km(), sort, area, &mut buf,
        );
        let header = row_text(&buf, 1, area);
        assert!(header.contains("RATING ▼"), "active RATING column shows the arrow: {header:?}");
        assert!(!header.contains("YEAR ▼") && !header.contains("TITLE ▼"), "{header:?}");
    }

    /// The RATE column is the first to go as the pane narrows — title and
    /// author must never be crowded out for it (SQ-0529's sizing brief).
    #[test]
    fn rating_column_drops_before_year_on_a_narrow_pane() {
        // Widths are in the same units `compute_columns` takes: the row space
        // left once the badge cluster and TYPE column are excluded.
        let wide = super::compute_columns(90, 20);
        assert!(wide.rating_w > 0 && wide.year_w > 0, "both shown when there is room");

        let mid = super::compute_columns(46, 20);
        assert_eq!(mid.rating_w, 0, "rating goes first");
        assert!(mid.year_w > 0, "year survives it");

        let narrow = super::compute_columns(34, 20);
        assert_eq!((narrow.rating_w, narrow.year_w), (0, 0), "then year");
        assert!(narrow.author_w > 0, "author outlives both");
    }

    /// SQ-1227's footer, at a width that holds all of it. This is the spec.
    const FOOTER: &str = "Enter: open  Space: menu  Tab: info  /: IFDB  \
                          g: covers  s: sort  r: refresh  Ctrl+F: find  ?: keys  q: quit";

    #[test]
    fn the_wide_footer_is_the_library_level_keys_one_key_each() {
        let km = km();
        assert_eq!(super::build_footer(&km, 200, false).trim(), FOOTER);
        // The gallery is the same line with `g` naming where it goes.
        let gallery = super::build_footer(&km, 200, true);
        assert_eq!(gallery.trim(), FOOTER.replace("g: covers", "g: list"));
        // …and neither carries navigation, a mouse gesture, or a per-story
        // action: those are the story menu's now.
        for gone in ["move", "page", "ends", "2×click", "2×right-click", "fetch", "get hints"] {
            assert!(!gallery.contains(gone), "{gone:?} is no longer a footer hint: {gallery:?}");
        }
    }

    /// The footer drops right-to-left by `drop_rank` — find first, keys last —
    /// and never drops open, menu or quit. (Was
    /// `footer_hints_drop_right_to_left_keeping_f_and_r_longest`; the premise
    /// changed with SQ-1227, the property did not.)
    #[test]
    fn footer_hints_drop_in_priority_order_keeping_open_menu_and_quit() {
        let km = km();
        // Narrow: none of the droppable hints fit, and the three that can never
        // be dropped are all still there.
        let narrow = super::build_footer(&km, 34, false);
        assert_eq!(narrow.trim(), "Enter: open  Space: menu  q: quit", "{narrow:?}");

        // Each optional segment's minimum fitting width is >= the previous
        // one's, walking them in KEEP order — which is the drop order reversed.
        // Robust to the segment set changing; what it pins is that there IS an
        // order and the footer honours it.
        let min_width = |seg: &str| -> u16 {
            (10u16..=200)
                .find(|&w| super::build_footer(&km, w, false).contains(seg))
                .unwrap_or(u16::MAX)
        };
        let optional = super::footer_optional(&km, false);
        assert_eq!(optional.first().map(String::as_str), Some("?: keys"), "last to go");
        assert_eq!(optional.last().map(String::as_str), Some("Ctrl+F: find"), "first to go");
        let widths: Vec<u16> = optional.iter().map(|s| min_width(s)).collect();
        for pair in widths.windows(2) {
            assert!(pair[0] <= pair[1], "segments appear in keep order: {widths:?}");
        }
        assert!(widths[0] < *widths.last().unwrap(), "{widths:?}");

        // And at every width in between, the three anchors survive and the
        // display order never rearranges itself.
        for w in 20u16..=200 {
            let f = super::build_footer(&km, w, false);
            assert!(f.contains("Enter: open"), "w={w}: {f:?}");
            assert!(f.contains("Space: menu"), "w={w}: {f:?}");
            assert!(f.contains("q: quit"), "w={w}: {f:?}");
            let shown: Vec<&str> = f.trim().split("  ").collect();
            let mut expected: Vec<&str> = FOOTER.split("  ").filter(|s| shown.contains(s)).collect();
            expected.dedup();
            assert_eq!(shown, expected, "display order is fixed at w={w}");
        }
    }

    // ── The per-story menu (SQ-1227) ────────────────────────────────────────

    /// `Space` on the highlighted row opens that story's menu, and nothing else.
    #[test]
    fn space_opens_the_story_menu_for_the_highlighted_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let km = km();
        assert_eq!(
            app::browser::action_for_key(&km, KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(app::browser::BrowserAction::OpenStoryMenu)
        );
        // The picker opens it on whatever row is selected, and the menu carries
        // that row so a later redraw anchors on the same story.
        let menu = app::story_menu::StoryMenu::new(7);
        assert_eq!(menu.story, 7);
        assert_eq!(menu.cursor, 0, "the menu opens on `Open`");
    }

    /// A SINGLE right-click on another row selects it AND opens its menu —
    /// there is no second-click state to get wrong, and a folder gets the
    /// selection without a menu it has no items for.
    #[test]
    fn a_single_right_click_selects_the_row_and_opens_its_menu() {
        assert_eq!(super::right_click_action(Some((4, false))), (Some(4), Some(4)));
        assert_eq!(super::right_click_action(Some((0, true))), (Some(0), None), "a folder");
        assert_eq!(super::right_click_action(None), (None, None), "past the rows: dismiss");
    }

    /// **SQ-0789's double right-click is gone** (SQ-1227): the launch-options
    /// dialog is a MENU ITEM now, so the right button's handler carries no
    /// double-click recogniser at all. Read off the source, because the property
    /// is the absence of code — no runtime assertion can see a gesture that was
    /// removed.
    #[test]
    fn the_right_button_no_longer_recognises_a_double_click() {
        let src = include_str!("picker_ui.rs");
        let start = src
            .find("if let MouseEventKind::Down(MouseButton::Right) = m.kind {")
            .expect("the right-button handler");
        let end = start
            + src[start..]
                .find("} else if let MouseEventKind::Down(MouseButton::Left)")
                .expect("the left-button handler after it");
        let region = &src[start..end];
        assert!(region.len() > 300, "the markers must bracket the real handler");
        for banned in ["DOUBLE_CLICK", "last_right_click", "open_launch_options"] {
            assert!(
                !region.contains(banned),
                "the right button must be a single click that opens the story menu; \
                 `{banned}` means SQ-0789's double-click gesture came back (SQ-1227)"
            );
        }
        assert!(region.contains("right_click_action"), "…through the one total function");
    }

    /// The menu's rows reach the very commands the picker's one dispatch runs —
    /// `Enter` on the highlighted row, and an item's own hotkey from anywhere.
    #[test]
    fn a_menu_item_dispatches_its_registry_command() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use app::browser::BrowserAction;
        use app::story_menu::{MenuOutcome, StoryMenu};
        let km = km();
        let plain = |c| KeyEvent::new(c, KeyModifiers::NONE);

        // Enter on "Launch options…".
        let mut menu = StoryMenu::new(0);
        menu.cursor = 1;
        let MenuOutcome::Activate(cmd) = menu.on_key(plain(KeyCode::Enter), &km) else {
            panic!("Enter activates the highlighted item");
        };
        assert_eq!(cmd, "open-launch-options");
        assert_eq!(app::browser::action_for_command(cmd), Some(BrowserAction::OpenLaunchOptions));

        // `f` from the top of the menu goes straight to the fetch.
        let mut menu = StoryMenu::new(0);
        let MenuOutcome::Activate(cmd) = menu.on_key(plain(KeyCode::Char('f')), &km) else {
            panic!("an item's own hotkey activates it");
        };
        assert_eq!(cmd, "fetch-story");
        assert_eq!(app::browser::action_for_command(cmd), Some(BrowserAction::FetchStory));

        // Esc closes without running anything.
        let mut menu = StoryMenu::new(0);
        assert_eq!(menu.on_key(plain(KeyCode::Esc), &km), MenuOutcome::Close);
    }

    /// The menu never spills off the pane, including on the bottom row — where
    /// it flips above the story instead of being clipped away.
    #[test]
    fn the_story_menu_is_clamped_inside_the_pane() {
        let km = km();
        let cs = app::colors::ColorScheme::terminal_default();
        let pane = ratatui::layout::Rect::new(0, 0, 60, 18);
        let mut buf = ratatui::buffer::Buffer::empty(pane);
        // A row on the last usable line of the pane.
        let anchor = ratatui::layout::Rect::new(1, 16, 50, 1);
        let menu = app::story_menu::StoryMenu::new(0);
        let rects = app::story_menu::draw_story_menu(&menu, anchor, pane, &km, &cs, &mut buf);
        assert!(rects.area.bottom() <= pane.bottom(), "{:?}", rects.area);
        assert!(rects.area.right() <= pane.right(), "{:?}", rects.area);
        assert!(rects.area.y < anchor.y, "flipped above the row: {:?}", rects.area);
        for (_, r) in &rects.items {
            assert!(pane.contains(ratatui::layout::Position { x: r.x, y: r.y }), "{r:?}");
        }
        // Every item is on screen and readable.
        let text = buffer_to_string(&buf, rects.area);
        for it in app::story_menu::STORY_MENU {
            assert!(text.contains(it.label), "{:?} missing: {text}", it.label);
        }
    }

    /// SQ-0796: the footer's keys come from the keymap, so rebinding one moves
    /// the hint with it — the drift a hand-written string could not survive.
    #[test]
    fn footer_hints_follow_a_rebinding() {
        let mut cfg = app::config::KeymapConfig::default();
        // `x` takes sort-library, and `s` is given away so the default binding
        // is displaced rather than joined.
        cfg.browser.insert("s".into(), "reverse-sort".into());
        cfg.browser.insert("x".into(), "sort-library".into());
        let (km, warns) = app::keymap::KeyMap::resolve(&cfg);
        assert!(warns.is_empty(), "{warns:?}");
        let wide = super::build_footer(&km, 200, false);
        assert!(wide.contains("x: sort"), "the user's key is advertised: {wide:?}");
        assert!(!wide.contains("s: sort"), "…and the displaced one is not: {wide:?}");
    }

    // ── Story-picker info panel ─────────────────────────────────────────────────

    fn buffer_to_string(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> String {
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell((x, y)) {
                    out.push_str(c.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn save_when_formats_date_and_time_of_day() {
        // SQ-0411: full RFC3339 → "YYYY-MM-DD HH:MM"; short/legacy fall back gracefully.
        assert_eq!(super::save_when("2026-07-19T13:05:42Z"), "2026-07-19 13:05");
        assert_eq!(super::save_when("2026-07-19"), "2026-07-19");
        assert_eq!(super::save_when(""), "");
    }

    /// SQ-0441: the info panel now draws its chrome through `draw_panel`, so it
    /// gets the default single-border frame and a centered, bracketed `Info`
    /// title on the top row (was a hardcoded frame + left-aligned " Info ").
    #[test]
    fn info_panel_frame_is_single_bordered_with_bracketed_title() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = app::picker::StoryMeta {
            size_bytes: 0, story_bytes: 0, modified: None, engine: app::picker::Engine::ZCode,
            format: "Z-code".into(), version: Some("3".into()), serial: None, release: None,
            ifid: "ZCODE-88-840726".into(), features: app::picker::Features::default(),
            self_blorb: None, disk_image: None, disk_entry: None, author: None, year: None, genre: None, language: None,
            description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        // Single-border top-left corner (BorderStyle::Single default).
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┌");
        // Top row carries the centered, bracket-capped title.
        let mut top = String::new();
        for x in area.left()..area.right() {
            top.push_str(buf.cell((x, 0)).unwrap().symbol());
        }
        assert!(top.contains("┤ Info ├"), "bracketed title on top row: {top:?}");
    }

    #[test]
    fn info_panel_renders_metadata_features_resources_and_saves() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = app::picker::StoryMeta {
            size_bytes: 92 * 1024, story_bytes: 92 * 1024,
            modified: Some("2026-06-30".into()),
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: Some("840726".into()),
            release: Some(88),
            ifid: "ZCODE-88-840726".into(),
            features: app::picker::Features { sound: true, graphics: true, colour: Some(false), hints: true },
            disk_image: None,
            disk_entry: None,
            self_blorb: Some(vec![
                app::picker::ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 92 * 1024, detail: None },
                app::picker::ChunkInfo {
                    usage: "Snd ".into(), number: 32, chunk_type: "FORM".into(), len: 12 * 1024,
                    detail: Some("15.4 kHz · 8-bit · mono · 2.2s".into()),
                },
            ]),
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        let game_dir = std::path::PathBuf::from("/tmp/lanthorn-info-panel-saves/zork1.z3");
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("before-troll.lanthorn"),
                name: "before-troll".into(),
                turns: 42,
                saved_at: "2026-06-30T13:05:00Z".into(),
                location: Some("The Troll Room".into()),
                score: Some(10),
                is_default: false, trigger: app::archive::SaveTrigger::HostState,
            }],
            hints_available: false,
            game_dir: game_dir.clone(),
            qzl_saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("quick.qzl"),
                name: "quick".into(),
                turns: 0,
                saved_at: "2026-06-29T00:00:00Z".into(),
                location: None,
                score: None,
                is_default: false, trigger: app::archive::SaveTrigger::HostState,
            }],
            auto_saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("_startup.qzl"),
                name: "_startup".into(),
                turns: 0,
                saved_at: "2026-06-28T00:00:00Z".into(),
                location: None,
                score: None,
                is_default: false, trigger: app::archive::SaveTrigger::HostState,
            }],
            sidecars: vec!["default.aux"],
            art_candidates: vec![],
            art_in_use: None,
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: Vec::new(),
        };
        // Wide enough that the resource detail suffix and the save-summary row aren't clipped.
        let area = Rect::new(0, 0, 100, 25);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, Some(&aux), 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );

        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Zork I"), "title line: {text:?}");
        assert!(text.contains("zork1.z3"), "filename: {text:?}");
        assert!(text.contains("Z-code"), "format line: {text:?}");
        assert!(text.contains("Release 88"));
        assert!(text.contains("840726"));
        assert!(text.contains("ZCODE-88-840726"));
        assert!(text.contains("sound"));
        assert!(text.contains("graphics"));
        assert!(text.contains("hints"));
        assert!(text.contains("Code"));
        assert!(text.contains("Sound"));
        assert!(text.contains("AIFF"));
        assert!(text.contains("15.4 kHz · 8-bit · mono · 2.2s"), "parsed detail: {text:?}");
        assert!(text.contains("Saves ·"), "saves dir header: {text:?}");
        assert!(text.contains("before-troll.lanthorn"), "lanthorn filename: {text:?}");
        // SQ-0411: the save summary surfaces location, score, and date + time-of-day.
        assert!(text.contains("The Troll Room"), "save location: {text:?}");
        assert!(text.contains("score 10"), "save score: {text:?}");
        assert!(text.contains("2026-06-30 13:05"), "save date + time-of-day: {text:?}");
        assert!(text.contains("quick.qzl"), "qzl filename: {text:?}");
        assert!(text.contains("Sidecars:"), "sidecars line: {text:?}");
        assert!(text.contains("default.aux"), "sidecar filename: {text:?}");
        // SQ-0285-b: auto (game-managed) saves render, clearly labeled.
        assert!(text.contains("(auto)"), "auto-save label: {text:?}");
        assert!(text.contains("_startup.qzl"), "auto-save filename: {text:?}");
        // SQ-0285-b: Saves section now renders ABOVE Resources.
        let saves_pos = text.find("Saves ·").expect("saves header present");
        let resources_pos = text.find("Resources").expect("resources header present");
        assert!(saves_pos < resources_pos, "Saves must render before Resources: saves@{saves_pos} resources@{resources_pos}");
    }

    // ───────────────────────── SQ-0861: info-panel wrapping ─────────────────
    //
    // The panel drew one row per logical line, so any value wider than it was
    // clipped at the edge. These pin that long values now wrap, that the scroll
    // arithmetic counts WRAPPED ROWS rather than logical lines, and that a panel
    // too narrow to wrap into still terminates.

    /// Collapse a string's whitespace runs to single spaces.
    fn norm(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The info panel's content as ONE whitespace-normalised string: border
    /// stripped, the continuation marker removed, rows joined.
    ///
    /// Word-wrap consumes the space it breaks on, so a wrapped line rejoins to
    /// exactly its original text under this normalisation — and a CLIPPED line
    /// cannot, because the clipped tail is nowhere in the buffer. Asserting here
    /// rather than on the string vector is the point: the defect was on screen.
    fn panel_text_flat(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> String {
        let mut out = String::new();
        for y in area.top() + 1..area.bottom().saturating_sub(1) {
            let mut row = String::new();
            for x in area.left() + 1..area.right().saturating_sub(1) {
                if let Some(c) = buf.cell((x, y)) {
                    row.push_str(c.symbol());
                }
            }
            out.push(' ');
            out.push_str(row.trim_start_matches(super::PANEL_CONT_MARK));
        }
        norm(&out)
    }

    /// The panel's text rows verbatim (border stripped, marker left in place).
    fn panel_rows(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> Vec<String> {
        (area.top() + 1..area.bottom().saturating_sub(1))
            .map(|y| {
                (area.left() + 1..area.right().saturating_sub(1))
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
                    .collect::<String>()
            })
            .collect()
    }

    /// The reported compilation case, as `stories/` actually holds it.
    fn compilation_meta() -> app::picker::StoryMeta {
        app::picker::StoryMeta {
            size_bytes: 819_200,
            story_bytes: 178_432,
            modified: Some("2026-08-14".into()),
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: Some("860730".into()),
            release: Some(59),
            // A UUID-form IFID: 36 chars behind a 5-char label is 41 columns,
            // past a 38-column panel, so this line was clipped too.
            ifid: "1D2E3F45-6789-4ABC-8DEF-0123456789AB".into(),
            features: app::picker::Features::default(),
            disk_image: None,
            disk_entry: Some("LEATHRGODDESSES".into()),
            self_blorb: None,
            author: None, year: None, genre: None, language: None, description: None,
            ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        }
    }

    const COMPILATION_FILE: &str =
        "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg";

    /// SQ-0861 (the reported defect): a compilation row's file line —
    /// `…(Disk 6 of 7).2mg:LEATHRGODDESSES` — is far wider than the panel and
    /// used to stop dead at its edge, taking the `:LEATHRGODDESSES` suffix that
    /// says WHICH of the five games this row is with it. Every character of it
    /// must now be on screen, across as many rows as it takes.
    #[test]
    fn info_panel_wraps_a_long_compilation_file_line() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = compilation_meta();
        // 40 columns is the info panel on a 120-column terminal (area.width / 3),
        // i.e. 38 columns of content — less than half the file line.
        let area = Rect::new(0, 0, 40, 30);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new(COMPILATION_FILE);
        super::draw_info_panel(
            "Leather Goddesses of Phobos", COMPILATION_FILE, &meta, None, 0, area, None,
            &mut cover, entry_path, entry_path, false, None, &cs, &mut buf,
            &mut Vec::new(), &mut Vec::new(),
        );
        let flat = panel_text_flat(&buf, area);
        // The filename has a line of its own now, and still needs three rows of
        // this panel — which is the case this test exists for.
        let expected = norm(&format!("{COMPILATION_FILE}:LEATHRGODDESSES"));
        assert!(flat.contains(&expected), "file line must render whole:\n  want {expected:?}\n  got  {flat:?}");
        // …and the sizes, on the line below it, are whole too.
        let sizes = norm(&format!(
            "{} · story {}",
            super::human_size(meta.size_bytes),
            super::human_size(meta.story_bytes),
        ));
        assert!(flat.contains(&sizes), "size line must render whole:\n  want {sizes:?}\n  got  {flat:?}");
        // The IFID was clipped by the same flat-line treatment.
        assert!(
            flat.contains("IFID 1D2E3F45-6789-4ABC-8DEF-0123456789AB"),
            "IFID must render whole: {flat:?}"
        );
        // Nothing spills past the panel's content width.
        for (i, row) in panel_rows(&buf, area).iter().enumerate() {
            assert!(
                app::textwidth::str_cells(row.trim_end()) <= (area.width - 2) as usize,
                "row {i} exceeds the panel's content width: {row:?}"
            );
        }
    }

    /// SQ-0861: a continuation row is set in behind its own marker, so a wrapped
    /// tail reads as more of the field above rather than as a new field — and
    /// the wrapped text keeps the style of the line it came from.
    #[test]
    fn info_panel_marks_and_indents_a_wrapped_continuation() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = compilation_meta();
        let area = Rect::new(0, 0, 40, 30);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new(COMPILATION_FILE);
        super::draw_info_panel(
            "Leather Goddesses of Phobos", COMPILATION_FILE, &meta, None, 0, area, None,
            &mut cover, entry_path, entry_path, false, None, &cs, &mut buf,
            &mut Vec::new(), &mut Vec::new(),
        );
        let rows = panel_rows(&buf, area);
        // Row 0 is the title; row 1 opens the file line, rows 2+ continue it.
        assert!(rows[1].starts_with("Lost Treasures"), "row 1: {:?}", rows[1]);
        assert!(rows[2].starts_with(super::PANEL_CONT_MARK), "row 2 must be marked: {:?}", rows[2]);
        assert!(!rows[1].starts_with(super::PANEL_CONT_MARK), "the first row of a field is not marked");
        // The marker carries `story_info_continuation`; the text beside it keeps
        // the file line's own `story_info_value`.
        // Foregrounds only: the panel paints its own background under every row,
        // so a cell's style is the selector's patched onto that fill.
        let mark = cs.theme.get("story_info_continuation").style.fg;
        let value = cs.theme.get("story_info_value").style.fg;
        assert_ne!(mark, value, "the marker must be distinguishable from the text it precedes");
        assert_eq!(buf.cell((area.x + 1, area.y + 3)).unwrap().fg, mark.unwrap(), "marker style");
        assert_eq!(
            buf.cell((area.x + 1 + super::PANEL_CONT_INDENT as u16, area.y + 3)).unwrap().fg,
            value.unwrap(),
            "wrapped text keeps its logical line's style",
        );
    }

    /// SQ-0861 (guard 1): a panel whose values all fit is laid out exactly as it
    /// was before wrapping existed — no continuation markers, no re-flow.
    #[test]
    fn info_panel_leaves_short_values_alone() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = app::picker::StoryMeta {
            size_bytes: 92 * 1024, story_bytes: 92 * 1024,
            modified: Some("2026-06-30".into()),
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: Some("840726".into()),
            release: Some(88),
            ifid: "ZCODE-88-840726".into(),
            features: app::picker::Features::default(),
            disk_image: None, disk_entry: None, self_blorb: None,
            author: None, year: None, genre: None, language: None, description: None,
            ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        let max_scroll = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 0, area, None, &mut cover, entry_path,
            entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        assert_eq!(max_scroll, 0, "content that fits must not become scrollable");
        let rows = panel_rows(&buf, area);
        assert!(
            !rows.iter().any(|r| r.starts_with(super::PANEL_CONT_MARK)),
            "no field should wrap at 60 columns: {rows:?}"
        );
        assert_eq!(rows[0].trim_end(), "Zork I");
        // Filename and sizes on their own lines, and no mtime anywhere: it dates
        // the file rather than the game, and sat next to a release and serial
        // that genuinely do.
        assert_eq!(rows[1].trim_end(), "zork1.z3");
        assert_eq!(rows[2].trim_end(), "92 KB");
        assert_eq!(rows[3].trim_end(), "Z-code v3 · Release 88");
        assert!(
            !rows.iter().any(|r| r.contains("2026-06-30")),
            "the file's mtime must not be shown: {rows:?}"
        );
    }

    /// SQ-0861 (guard 3): with wrapped content present, `max_scroll` is measured
    /// in WRAPPED ROWS, so scrolling reaches the true last row — and the
    /// scrollbar agrees, its thumb landing on the track's bottom cell exactly
    /// there. Counting logical lines instead leaves the tail unreachable.
    #[test]
    fn info_panel_scroll_reaches_the_last_wrapped_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let scrollbar = app::render::scroll::ScrollbarLook::from_theme(&cs.theme);
        assert_ne!(scrollbar.thumb, scrollbar.track, "thumb and track must differ for this assertion to bite");
        // Twelve resource rows, each long enough to wrap into three: 36 logical
        // lines' worth of content occupying far more rows than that.
        let chunks: Vec<app::picker::ChunkInfo> = (0..12)
            .map(|i| app::picker::ChunkInfo {
                usage: "Snd ".into(),
                number: i,
                chunk_type: "FORM".into(),
                len: 128,
                detail: Some(format!("sampled at 44.1 kHz · 16-bit · stereo · loop point {i} · 12.5s")),
            })
            .collect();
        let mut meta = compilation_meta();
        meta.self_blorb = Some(chunks);
        let area = Rect::new(0, 0, 40, 12);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new(COMPILATION_FILE);
        let mut render = |scroll: usize, buf: &mut Buffer| {
            super::draw_info_panel(
                "Leather Goddesses of Phobos", COMPILATION_FILE, &meta, None, scroll, area, None,
                &mut cover, entry_path, entry_path, false, None, &cs, buf,
                &mut Vec::new(), &mut Vec::new(),
            )
        };

        let mut buf_top = Buffer::empty(area);
        let max_scroll = render(0, &mut buf_top);
        assert!(max_scroll > 0, "wrapped content must overflow a 12-row panel");
        // The final resource's tail is the last thing the panel has to show.
        let tail = "loop point 11 · 12.5s";
        assert!(!panel_text_flat(&buf_top, area).contains(tail), "the tail must start offscreen");
        // Scrollbar at the top: the track's bottom cell is not the thumb.
        let sb_x = area.right() - 2;
        let sb_bottom = area.bottom() - 2;
        assert_eq!(buf_top.cell((sb_x, sb_bottom)).unwrap().bg, scrollbar.track, "thumb must not be at the bottom at scroll 0");

        let mut buf_end = Buffer::empty(area);
        assert_eq!(render(max_scroll, &mut buf_end), max_scroll, "max_scroll is stable across scroll positions");
        let rows_end = panel_rows(&buf_end, area);
        assert!(
            panel_text_flat(&buf_end, area).contains(tail),
            "the last wrapped row must be reachable: {rows_end:?}"
        );
        // At max_scroll the last row of content sits on the panel's bottom row —
        // if max_scroll counted logical lines it would be too small and the
        // bottom rows would still be blank here.
        assert!(!rows_end.last().unwrap().trim().is_empty(), "no blank rows below the content at max_scroll: {rows_end:?}");
        assert_eq!(buf_end.cell((sb_x, sb_bottom)).unwrap().bg, scrollbar.thumb, "the thumb must reach the track bottom at max_scroll");

        // One row short of the end still hides the very last row.
        let mut buf_near = Buffer::empty(area);
        render(max_scroll - 1, &mut buf_near);
        assert_ne!(
            panel_rows(&buf_near, area).last().unwrap(),
            rows_end.last().unwrap(),
            "max_scroll must be the FIRST scroll that shows the last row"
        );
    }

    /// SQ-0861: a link too wide for the panel wraps like any other value, and
    /// every row it wraps onto stays a link to the WHOLE URL — a wrapped tail
    /// must not become dead text the click path has no rect for.
    #[test]
    fn info_panel_keeps_a_wrapped_link_clickable_on_every_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let url = "https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro";
        let mut meta = minimal_story_meta();
        meta.ifdb_link = Some(url.into());
        // 30 columns of content: `IFDB: ` + a 45-column URL cannot fit on one row.
        let area = Rect::new(0, 0, 32, 20);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut links: Vec<(Rect, String)> = Vec::new();
        super::draw_info_panel(
            "Game", "game.z5", &meta, None, 0, area, None, &mut cover,
            std::path::Path::new("game.z5"), std::path::Path::new("game.z5"),
            false, None, &cs, &mut buf, &mut links, &mut Vec::new(),
        );
        assert!(links.len() > 1, "the link must wrap onto more than one row: {links:?}");
        assert!(links.iter().all(|(_, u)| u == url), "every row opens the full URL: {links:?}");
        // Consecutive rows of one field, the continuation set in behind the marker.
        let (first, second) = (links[0].0, links[1].0);
        assert_eq!(second.y, first.y + 1, "the rows are adjacent");
        assert_eq!(second.x, first.x + super::PANEL_CONT_INDENT as u16, "the tail is indented");
        assert_eq!(
            panel_rows(&buf, area)[second.y as usize - 1].chars().next(),
            super::PANEL_CONT_MARK.chars().next(),
            "the wrapped link row carries the continuation marker",
        );
    }

    /// SQ-0861 (guard 4): a panel with almost no content width must not panic or
    /// spin. One column of content, and a content width narrower than the single
    /// wide glyph it has to place, both have to terminate.
    #[test]
    fn info_panel_survives_a_degenerate_width() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = compilation_meta();
        meta.serial = Some("見テ".into());
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new(COMPILATION_FILE);
        // width 2 → 0 content columns; 3 → 1; 4 → 2 (too narrow for the indent).
        for w in 2u16..=6 {
            let area = Rect::new(0, 0, w, 6);
            let mut buf = Buffer::empty(area);
            super::draw_info_panel(
                "宇宙船の物語", COMPILATION_FILE, &meta, None, 0, area, None, &mut cover,
                entry_path, entry_path, false, None, &cs, &mut buf,
                &mut Vec::new(), &mut Vec::new(),
            );
        }
    }

    /// SQ-0861: wrapping measures TERMINAL COLUMNS, not bytes or chars — a CJK
    /// glyph is two cells and is never split in half — and every call advances,
    /// including when nothing fits at all.
    #[test]
    fn wrap_panel_line_measures_columns_and_always_advances() {
        // Pure ASCII, word-wrapped: the break space is consumed, words stay whole.
        assert_eq!(super::wrap_panel_line("alpha beta gamma", 11, 11), vec!["alpha beta", "gamma"]);
        // A token wider than the row is BROKEN, not left for the renderer to
        // clip — clipping is the defect. Every character survives.
        assert_eq!(super::wrap_panel_line("LEATHRGODDESSES", 6, 6), vec!["LEATHR", "GODDES", "SES"]);
        // The continuation width is what rows after the first use.
        assert_eq!(super::wrap_panel_line("abcdefghij", 6, 3), vec!["abcdef", "ghi", "j"]);
        // CJK: two cells each, so four fit in a 9-column row, not nine, and the
        // fifth moves whole rather than being split down the middle.
        assert_eq!(super::wrap_panel_line("宇宙船の物語", 9, 9), vec!["宇宙船の", "物語"]);
        // A run of spaces at a break point does not become a row of blanks (the
        // panel's own save rows use double spaces as column separators).
        assert_eq!(super::wrap_panel_line("turn 42  save.lanthorn", 8, 8), vec!["turn 42", "save.lan", "thorn"]);
        // Nothing fits at all: the glyph is taken anyway so the scan advances.
        assert_eq!(super::wrap_panel_line("宇宙", 1, 1), vec!["宇", "宙"]);
        // Zero width cannot make progress; the line comes back unwrapped.
        assert_eq!(super::wrap_panel_line("anything", 0, 0), vec!["anything"]);
        // Every character of the input survives, whatever the width.
        for w in 1..12usize {
            let joined: String = super::wrap_panel_line("宇宙船の物語 abc", w, w).concat();
            assert_eq!(joined.replace(' ', ""), "宇宙船の物語abc", "width {w} lost characters");
        }
    }

    #[test]
    fn info_panel_scrolls_to_reveal_overflow() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let chunks: Vec<app::picker::ChunkInfo> = (0..30)
            .map(|i| app::picker::ChunkInfo {
                usage: "Data".into(),
                number: i,
                chunk_type: "IFhd".into(),
                len: 128,
                detail: None,
            })
            .collect();
        let meta = app::picker::StoryMeta {
            size_bytes: 92 * 1024, story_bytes: 92 * 1024,
            modified: None,
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: None,
            release: None,
            ifid: "ZCODE-88-840726".into(),
            features: app::picker::Features::default(),
            self_blorb: Some(chunks),
            disk_image: None,
            disk_entry: None,
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 34, 10);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        let max_scroll = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text_top = buffer_to_string(&buf, area);
        assert!(max_scroll > 0, "content should overflow a 10-row panel");
        let late_marker = " #29  ";
        assert!(!text_top.contains(late_marker), "late resource should be offscreen at scroll 0: {text_top:?}");

        let mut buf2 = Buffer::empty(area);
        let max_scroll2 = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, max_scroll, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf2, &mut Vec::new(), &mut Vec::new(),
        );
        let text_scrolled = buffer_to_string(&buf2, area);
        assert_eq!(max_scroll2, max_scroll);
        assert!(text_scrolled.contains(late_marker), "late resource should be visible when scrolled: {text_scrolled:?}");

        // Scrolling past max clamps to the same view as scroll == max_scroll.
        let mut buf3 = Buffer::empty(area);
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 999, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf3, &mut Vec::new(), &mut Vec::new(),
        );
        let text_over = buffer_to_string(&buf3, area);
        assert_eq!(text_over, text_scrolled, "scroll past max should clamp to max_scroll view");
    }

    fn minimal_story_meta() -> app::picker::StoryMeta {
        app::picker::StoryMeta {
            size_bytes: 1, story_bytes: 1, modified: None, engine: app::picker::Engine::Glulx,
            format: "Blorb (Glulx)".into(), version: Some("3.1.2".into()),
            serial: None, release: None, ifid: "IFID-X".into(),
            features: app::picker::Features::default(), self_blorb: None, disk_image: None, disk_entry: None,
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        }
    }

    /// SQ-0771: the size on the filename line measures the file on disk, which
    /// for a container is not the game. The panel names the mounted story's own
    /// size beside it — and only then, so a plain story file's line is unchanged.
    #[test]
    fn info_panel_names_the_mounted_storys_size_for_a_container() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let render = |meta: &app::picker::StoryMeta, name: &str| {
            let area = Rect::new(0, 0, 70, 12);
            let mut buf = Buffer::empty(area);
            let mut cover = app::cover::CoverState::default();
            let entry_path = std::path::Path::new(name);
            super::draw_info_panel(
                "Zork I", name, meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None,
                &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
            );
            buffer_to_string(&buf, area)
        };

        // An Amiga release floppy: 880 KB of container around a 91 KB game.
        let mut adf = minimal_story_meta();
        adf.size_bytes = 901_120;
        adf.story_bytes = 93_766;
        adf.disk_image = Some(app::hints::DiskImage::Adf);
        let text = render(&adf, "Zork I - The Great Underground Empire.adf");
        assert!(text.contains("880 KB"), "the container's own size stays: {text:?}");
        assert!(text.contains("story 91 KB"), "the mounted story's size is named: {text:?}");

        // A plain story file: one size, no second segment.
        let mut bare = minimal_story_meta();
        bare.size_bytes = 93_766;
        bare.story_bytes = 93_766;
        let text = render(&bare, "zork1.z3");
        assert!(text.contains("91 KB"), "the size still renders: {text:?}");
        assert!(!text.contains("story 91 KB"), "no redundant second size: {text:?}");
    }

    // ── SQ-0348: author/year/genre + blurb ──────────────────────────────────────

    /// A story with NO fetched/embedded metadata must render exactly as it did
    /// before this feature existed: no empty "Author:" label, no stray blank
    /// line, no separator with nothing either side of it. The IFID line and
    /// the Features line (present since `minimal_story_meta` has no features by
    /// default, so give it one) must land on directly adjacent rows.
    #[test]
    fn info_panel_no_metadata_leaves_ifid_and_features_adjacent() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.features = app::picker::Features { sound: true, ..Default::default() };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        let lines: Vec<&str> = text.lines().collect();
        // The property is ADJACENCY, not a row number: with no metadata, nothing
        // may be inserted between IFID and Features. Found rather than indexed,
        // so a layout change above them (the filename and its sizes became two
        // lines) moves the pair without falsely failing this.
        let ifid = lines
            .iter()
            .position(|l| l.contains("IFID"))
            .unwrap_or_else(|| panic!("the IFID line should be shown: {text:?}"));
        assert!(
            lines[ifid + 1].trim_start_matches('│').trim().starts_with("Features:"),
            "no line should be inserted between IFID and Features when metadata is absent: {:?}",
            lines[ifid + 1]
        );
        assert!(!text.contains("Author"), "no metadata label should appear: {text:?}");
    }

    /// With author/year/genre and a blurb present, a combined "author · year ·
    /// genre" line and the wrapped blurb text land between IFID and Features,
    /// disturbing neither.
    #[test]
    fn info_panel_renders_author_year_genre_and_blurb_between_ifid_and_features() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.author = Some("Michael S. Gentry".into());
        meta.year = Some("1998".into());
        meta.genre = Some("Horror".into());
        meta.description = Some("A tale of terror in a small town.".into());
        meta.features = app::picker::Features { sound: true, ..Default::default() };
        let area = Rect::new(0, 0, 50, 14);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Michael S. Gentry"), "author should render: {text:?}");
        assert!(text.contains("1998"), "year should render: {text:?}");
        assert!(text.contains("Horror"), "genre should render: {text:?}");
        assert!(text.contains("A tale of terror in a small town."), "blurb should render: {text:?}");

        let ifid_pos = text.find("IFID").expect("IFID line present");
        let author_pos = text.find("Michael S. Gentry").expect("author present");
        let blurb_pos = text.find("A tale of terror").expect("blurb present");
        let features_pos = text.find("Features:").expect("features present");
        assert!(ifid_pos < author_pos, "author line must come after IFID");
        assert!(author_pos < blurb_pos, "blurb must come after the author/year/genre line");
        assert!(blurb_pos < features_pos, "blurb must come before Features");
    }

    /// SQ-0372: a story stored beside a separate resource `.blorb` names that
    /// sidecar file up-front in the metadata block (not only in the Resources
    /// header), so it is visible without scrolling.
    #[test]
    fn info_panel_names_the_sidecar_resource_blorb() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta(); // self_blorb: None → the sidecar path
        let aux = app::picker::StoryAux {
            assoc_blorb: Some((
                std::path::PathBuf::from("/games/beyondzork.blb"),
                vec![app::picker::ChunkInfo {
                    usage: "Pict".into(), number: 1, chunk_type: "PNG ".into(), len: 4096, detail: None,
                }],
            )),
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/bz"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: vec![],
            art_in_use: None,
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: Vec::new(),
        };
        let area = Rect::new(0, 0, 60, 16);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        super::draw_info_panel(
            "Beyond Zork", "beyondzork-r57-s871221.z5", &meta, Some(&aux), 0, area, None,
            &mut cover, std::path::Path::new("beyondzork-r57-s871221.z5"), std::path::Path::new("beyondzork-r57-s871221.z5"), false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Resource blorb: beyondzork.blb"), "sidecar named up-front: {text:?}");
        assert!(text.contains("Image"), "the sidecar's Pict resource still lists: {text:?}");
        let blorb_pos = text.find("Resource blorb:").expect("sidecar line present");
        let res_pos = text.find("Resources").expect("resources header present");
        assert!(blorb_pos < res_pos, "sidecar name is up-front, before the Resources section");
    }

    /// SQ-0789: the info panel lists the picture archives detected for the
    /// selected story — read-only, from the *same* `discover_art_candidates` the
    /// launch-options dialog offers, so the two surfaces cannot disagree about
    /// what the story has. Driven off the real library so the list is the real
    /// one; skips vacuously when `stories/` is absent (gitignored fixtures).
    #[test]
    fn info_panel_lists_the_archives_detected_for_this_story() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let z0 = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork0-r393-s890714.z6");
        if !z0.is_file() {
            return;
        }
        let candidates = app::launch_options::discover_art_candidates(&z0, None);
        assert!(!candidates.is_empty(), "Zork Zero's archives sit beside it");
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta();
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/zork0"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: candidates.clone(),
            // What the game's own config.toml names, so the panel can mark it.
            art_in_use: Some("zork0.mg1".into()),
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: Vec::new(),
        };
        // Tall enough that the block is on screen without scrolling.
        let area = Rect::new(0, 0, 62, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        super::draw_info_panel(
            "Zork Zero", "zork0-r393-s890714.z6", &meta, Some(&aux), 0, area, None,
            &mut cover, &z0, &z0, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        println!("{text}");

        assert!(text.contains("Artwork ·"), "the block has a header: {text:?}");
        // Every detected archive, with enough to tell the renditions apart.
        for c in &candidates {
            assert!(text.contains(&c.filename), "{} is listed: {text:?}", c.filename);
        }
        assert!(text.contains("MCGA") || text.contains("Amiga"), "flavour shown: {text:?}");
        assert!(text.contains("pictures"), "picture count shown: {text:?}");
        assert!(text.contains("← in use"), "the archive in force is marked: {text:?}");
        // And no other game's art, which is the filter the dialog now shares.
        assert!(!text.contains("arthur."), "another game's archives stay out: {text:?}");
        assert!(!text.contains("journey."), "{text:?}");
    }

    /// SQ-1018: the panel lists the typefaces the story's own medium carries and
    /// marks which one the renderer takes.
    ///
    /// **Synthetic on purpose.** Every real face lives on gitignored commercial
    /// media, so a fixture-driven case skips on CI — and this is the surface that
    /// would have made SQ-1018 visible on sight, so it is worth having a part of
    /// it CI can still see. `native_disk_font` drives the real disks.
    ///
    /// The claim that matters is the SECOND row: a face present and NOT drawn has
    /// to read differently from one that is, because "carried but unreached" is
    /// exactly the state the Masterpieces CD was in.
    #[test]
    fn info_panel_lists_the_faces_on_the_medium_and_marks_the_one_in_use() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta();
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/arthur"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: vec![],
            art_in_use: None,
            disk_sounds: Vec::new(),
            // Arthur's Macintosh pressing, as `native_font::detected` reports it.
            disk_fonts: vec![
                app::native_font::DiskFace {
                    name: "FONT 524".into(),
                    width: 7,
                    height: 15,
                    proportional: true,
                    used: true,
                },
                app::native_font::DiskFace {
                    name: "FONT 1033".into(),
                    width: 7,
                    height: 12,
                    proportional: false,
                    used: false,
                },
            ],
            system_fonts: Vec::new(),
        };
        let area = Rect::new(0, 0, 62, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let p = std::path::Path::new("arthur.z6");
        super::draw_info_panel(
            "Arthur", "arthur.z6", &meta, Some(&aux), 0, area, None, &mut cover, p, p, false,
            None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        println!("{text}");

        assert!(text.contains("Fonts on the medium (2)"), "the block has a header: {text:?}");
        let row_of = |name: &str| -> String {
            text.lines()
                .find(|l| l.contains(name))
                .unwrap_or_else(|| panic!("{name} is listed: {text:?}"))
                .to_string()
        };
        let body = row_of("FONT 524");
        assert!(body.contains("7x15"), "the cell it is drawn for: {body:?}");
        assert!(body.contains("in use"), "the body face is the one drawn: {body:?}");

        let alt = row_of("FONT 1033");
        assert!(alt.contains("7x12"), "its own cell: {alt:?}");
        assert!(
            !alt.contains("in use"),
            "a face the renderer does not reach must not read as one it does: {alt:?}",
        );
    }

    /// SQ-1038: the panel also lists typefaces off the user's OWN disks under
    /// `~/.lanthorn/`, named with the disk each came from and never marked "in
    /// use" — nothing renders with one of these yet (SQ-1037).
    ///
    /// Two rows share a name on purpose, matching the user's own Workbench 1.2
    /// and 1.3 disks (identical font drawers): both must still be listed rather
    /// than collapsing into one.
    #[test]
    fn info_panel_lists_system_fonts_named_with_their_disk_and_never_marks_them_in_use() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta();
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/arthur"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: vec![],
            art_in_use: None,
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: vec![
                app::system_fonts::SystemFace {
                    disk: "MacOS_6.0.8_System_Startup.img".into(),
                    name: "FONT 396".into(),
                    width: 24,
                    height: 24,
                    proportional: true,
                    machine: app::interpreter::InterpreterProfile::Macintosh,
                },
                app::system_fonts::SystemFace {
                    disk: "Workbench v1.2.adf".into(),
                    name: "fonts/garnet/16".into(),
                    width: 16,
                    height: 16,
                    proportional: true,
                    machine: app::interpreter::InterpreterProfile::Amiga,
                },
                app::system_fonts::SystemFace {
                    disk: "Workbench v1.3.adf".into(),
                    name: "fonts/garnet/16".into(),
                    width: 16,
                    height: 16,
                    proportional: true,
                    machine: app::interpreter::InterpreterProfile::Amiga,
                },
            ],
        };
        // Wider than the other panel fixtures: a disk filename is long enough
        // ("MacOS_6.0.8_System_Startup.img") that the default 62 wraps the row
        // and splits it from its own "proportional" tag onto a continuation
        // line the per-row lookup below would miss.
        let area = Rect::new(0, 0, 90, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let p = std::path::Path::new("arthur.z6");
        super::draw_info_panel(
            "Arthur", "arthur.z6", &meta, Some(&aux), 0, area, None, &mut cover, p, p, false,
            None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        println!("{text}");

        assert!(text.contains("System fonts (3)"), "the block has a header: {text:?}");
        let rows_of = |name: &str| -> Vec<String> {
            text.lines().filter(|l| l.contains(name)).map(str::to_string).collect()
        };

        // **Each disk is named ONCE, as a heading.** That is the whole point of
        // grouping: a system disk carries a whole drawer — eighteen faces off a
        // System 6.0.8 startup disk — and repeating a sixty-character filename on
        // every row buries the faces in their own provenance. Asserted as a COUNT,
        // because "appears at least once" would pass equally well for the
        // per-row repetition this replaced.
        for disk in ["MacOS_6.0.8_System_Startup.img", "Workbench v1.2.adf", "Workbench v1.3.adf"] {
            let named = rows_of(disk);
            assert_eq!(named.len(), 1, "{disk} is named exactly once: {named:?}");
        }

        // A face row carries its own metrics and NOT its disk — it sits under the
        // heading that already said so.
        let mac = rows_of("FONT 396");
        assert_eq!(mac.len(), 1, "one Macintosh face: {mac:?}");
        assert!(mac[0].contains("24x24") && mac[0].contains("proportional"), "{mac:?}");
        assert!(
            !mac[0].contains("MacOS_6.0.8"),
            "the face row does not repeat its disk: {mac:?}",
        );
        assert!(!mac[0].contains("in use"), "a system face is never marked in use: {mac:?}");

        // The duplicate: the SAME face name on two disks still reads as two,
        // because each sits under its own heading. Workbench 1.2 and 1.3 ship
        // identical font drawers, so collapsing them would be one list silently
        // standing for two.
        let garnet = rows_of("fonts/garnet/16");
        assert_eq!(garnet.len(), 2, "both Workbench disks list it: {garnet:?}");

        // And the headings carry a per-disk count, so a reader can see the split
        // without counting rows.
        assert!(text.contains("(1)"), "each disk heading states how many it carries: {text:?}");
    }

    /// SQ-0798: Arthur's split EGA set is one row in the panel, exactly as it is
    /// one row in the dialog, and the row says it is two disks.
    ///
    /// There is one discovery path by design, so this and the dialog cannot show
    /// different lists — but "cannot" is worth pinning, because the failure it
    /// prevents is a panel that offers `arthur.eg2` as if picking it were a
    /// sensible thing to do, when it is 101 of the set's 171 ids.
    #[test]
    fn info_panel_shows_a_split_ega_set_as_one_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let arthur = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/arthur-r74-s890714.z6");
        if !arthur.is_file() || !arthur.with_file_name("arthur.eg2").is_file() {
            return; // gitignored fixtures
        }
        let candidates = app::launch_options::discover_art_candidates(&arthur, None);
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta();
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/arthur"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: candidates,
            art_in_use: Some("arthur.eg1".into()),
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: Vec::new(),
        };
        let area = Rect::new(0, 0, 62, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        super::draw_info_panel(
            "Arthur", "arthur-r74-s890714.z6", &meta, Some(&aux), 0, area, None,
            &mut cover, &arthur, &arthur, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        println!("{text}");

        assert!(text.contains("arthur.eg1"), "the head of the set is listed: {text:?}");
        assert!(!text.contains("arthur.eg2"), "and its back half is not a row: {text:?}");
        assert!(text.contains("2 disks"), "the row says it carries both files: {text:?}");
        assert!(text.contains("171 pictures"), "and counts the whole set: {text:?}");
        assert!(text.contains("← in use"), "the archive in force is marked: {text:?}");
    }

    /// A `pictures` key naming something the name filter would never detect —
    /// the renamed `FMVPOKER.EG1` case, or an absolute path — is still what the
    /// game draws, so the panel says so rather than showing an empty block.
    #[test]
    fn info_panel_names_an_archive_the_filter_would_never_have_found() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta();
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/zork0"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
            art_candidates: vec![],
            art_in_use: Some("FMVPOKER.EG1".into()),
            disk_sounds: Vec::new(),
            disk_fonts: Vec::new(),
            system_fonts: Vec::new(),
        };
        let area = Rect::new(0, 0, 62, 30);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        super::draw_info_panel(
            "Zork Zero", "zork0.z6", &meta, Some(&aux), 0, area, None, &mut cover,
            std::path::Path::new("zork0.z6"), std::path::Path::new("zork0.z6"), false, None, &cs, &mut buf, &mut Vec::new(),
            &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Artwork · 0 detected"), "{text:?}");
        assert!(text.contains("in use: FMVPOKER.EG1"), "the named archive is named: {text:?}");
    }

    /// Regression (SQ-0367 link vs scrollbar): hyperrat leaves the OSC 8 first
    /// cell's diff option at None, so ratatui measures the escape sequence as the
    /// cell's width and skips the rest of the row — a stale label tail and a
    /// corrupted scrollbar. We must pin that cell to ForcedWidth(1).
    #[test]
    fn info_panel_link_cell_is_pinned_to_width_one() {
        use ratatui::buffer::{Buffer, CellDiffOption};
        use ratatui::layout::{Position, Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.ifdb_link = Some("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".into());
        let area = Rect::new(0, 0, 60, 14);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut links: Vec<(Rect, String)> = Vec::new();
        super::draw_info_panel(
            "Game", "game.z5", &meta, None, 0, area, None, &mut cover,
            std::path::Path::new("game.z5"), std::path::Path::new("game.z5"), false, None, &cs, &mut buf, &mut links, &mut Vec::new(),
        );
        let (rect, _) = links.first().expect("a link rect was recorded");
        let first = buf.cell(Position::new(rect.x, rect.y)).expect("link first cell");
        assert!(
            matches!(first.diff_option, CellDiffOption::ForcedWidth(w) if w.get() == 1),
            "link first cell must be pinned to width 1, was {:?}", first.diff_option
        );
    }

    #[test]
    fn info_panel_shows_the_ifdb_link_only_once_fetched() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let render = |meta: &app::picker::StoryMeta| {
            let mut buf = Buffer::empty(area);
            let mut cover = app::cover::CoverState::default();
            super::draw_info_panel(
                "Game", "game.z5", meta, None, 0, area, None, &mut cover,
                std::path::Path::new("game.z5"), std::path::Path::new("game.z5"), false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
            );
            buffer_to_string(&buf, area)
        };
        // Not fetched → no IFDB line at all.
        let bare = minimal_story_meta();
        assert!(!render(&bare).contains("IFDB:"), "no link before a fetch");
        // Fetched → the bare URL renders (terminals auto-link it).
        let mut fetched = minimal_story_meta();
        fetched.ifdb_link = Some("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".into());
        assert!(
            render(&fetched).contains("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro"),
            "the IFDB URL renders once present"
        );
    }

    /// SQ-0371: a fetch that found nothing offers a manual IFDB search link (by
    /// title) instead of a dead end — but a never-fetched story shows neither.
    #[test]
    fn info_panel_offers_a_search_link_when_the_fetch_found_nothing() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let render = |title: &str, meta: &app::picker::StoryMeta| {
            let mut buf = Buffer::empty(area);
            let mut cover = app::cover::CoverState::default();
            super::draw_info_panel(
                title, "game.z5", meta, None, 0, area, None, &mut cover,
                std::path::Path::new("game.z5"), std::path::Path::new("game.z5"), false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
            );
            buffer_to_string(&buf, area)
        };
        // Never fetched → no search link (only `f`/`r` offers a fetch).
        let bare = minimal_story_meta();
        assert!(!render("Zork I", &bare).contains("IFDB search:"), "no search link before a fetch");
        // Fetch ran, found nothing → a search-by-title link appears.
        let mut nf = minimal_story_meta();
        nf.fetch_not_found = true;
        let out = render("Zork I", &nf);
        assert!(out.contains("IFDB search:"), "not-found offers a manual search: {out:?}");
        assert!(out.contains("ifdb.org/search?searchfor=Zork"), "search is by title: {out:?}");
        // A successful link takes precedence over the search fallback.
        let mut found = minimal_story_meta();
        found.fetch_not_found = true; // even if the flag is stale
        found.ifdb_link = Some("https://ifdb.org/viewgame?id=abc".into());
        let out = render("Zork I", &found);
        assert!(out.contains("viewgame?id=abc") && !out.contains("IFDB search:"), "link wins: {out:?}");
    }

    /// SQ-0367: the info panel reports the screen rect + URL of each OSC 8 link
    /// it draws, so the picker loop can open one on a click (mouse capture keeps
    /// the terminal from acting on the hyperlink itself). No link → empty.
    #[test]
    fn info_panel_surfaces_the_link_rect_for_click_to_open() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let mut cover = app::cover::CoverState::default();
        let path = std::path::Path::new("game.z5");

        // No fetch → no link → no rects.
        let mut buf = Buffer::empty(area);
        let mut rects: Vec<(Rect, String)> = Vec::new();
        super::draw_info_panel(
            "Zork I", "game.z5", &minimal_story_meta(), None, 0, area, None, &mut cover,
            path, path, false, None, &cs, &mut buf, &mut rects, &mut Vec::new(),
        );
        assert!(rects.is_empty(), "no link rects before a fetch: {rects:?}");

        // Fetched → one link rect carrying the full URL, inside the panel.
        let mut fetched = minimal_story_meta();
        let url = "https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".to_string();
        fetched.ifdb_link = Some(url.clone());
        let mut buf = Buffer::empty(area);
        rects.clear();
        super::draw_info_panel(
            "Zork I", "game.z5", &fetched, None, 0, area, None, &mut cover,
            path, path, false, None, &cs, &mut buf, &mut rects, &mut Vec::new(),
        );
        assert_eq!(rects.len(), 1, "one link rect once fetched: {rects:?}");
        let (rect, got) = &rects[0];
        assert_eq!(got, &url, "rect carries the full URL for opening");
        assert!(area.contains(rect.as_position()), "link rect is inside the panel");
    }

    /// A long blurb wraps to the panel's content width and, when it overflows
    /// the panel height, scrolls with the SAME `panel_scroll`/`panel_max`
    /// mechanism as the rest of the info panel (no second scroll system).
    #[test]
    fn info_panel_blurb_wraps_and_participates_in_panel_scroll() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.description = Some(
            "one two three four five six seven eight nine ten eleven twelve \
             thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty"
                .into(),
        );
        // Narrow + short so the wrapped blurb both wraps to multiple lines and
        // overflows the panel height.
        let area = Rect::new(0, 0, 20, 8);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        let max_scroll = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        assert!(max_scroll > 0, "a long wrapped blurb should overflow an 8-row panel");
        let text_top = buffer_to_string(&buf, area);
        assert!(!text_top.contains("twenty"), "late blurb word should be offscreen at scroll 0: {text_top:?}");

        let mut buf2 = Buffer::empty(area);
        let max_scroll2 = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, max_scroll, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf2, &mut Vec::new(), &mut Vec::new(),
        );
        assert_eq!(max_scroll2, max_scroll, "max_scroll must be stable across scroll positions");
        let text_scrolled = buffer_to_string(&buf2, area);
        assert!(text_scrolled.contains("twenty"), "late blurb word should be visible once scrolled: {text_scrolled:?}");
    }

    /// Regression (word-wrap vs scrollbar): when the blurb overflows and the
    /// scrollbar claims the last column, the wrap must reserve that gutter so a
    /// word landing on the panel's right edge is not clipped by one character.
    #[test]
    fn info_panel_blurb_wrap_reserves_the_scrollbar_gutter() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        // In the 18-col inner width, "aaaaaaaa bbbbbbbbb" is exactly 18 wide: it
        // fills the full inner width but not the width-1 text area left once the
        // scrollbar takes a column. Wrapping to the full width clips the last
        // 'b'; reserving the gutter pushes "bbbbbbbbb" to its own line intact.
        meta.description = Some(
            "aaaaaaaa bbbbbbbbb cccccccc dddddddd eeeeeeee ffffffff gggggggg hhhhhhhh iiiiiiii jjjjjjjj"
                .into(),
        );
        let area = Rect::new(0, 0, 20, 10); // inner 18x8 → 10 content lines overflow
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        let max_scroll = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, entry_path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        assert!(max_scroll > 0, "blurb should overflow so the scrollbar shows");
        let text = buffer_to_string(&buf, area);
        assert!(
            text.contains("bbbbbbbbb"),
            "the 9-char word must not be clipped by the scrollbar column: {text:?}"
        );
    }

    #[test]
    fn info_panel_renders_cover_band_when_present() {
        use ratatui::layout::Rect;
        use ratatui::buffer::Buffer;

        // A tiny valid PNG (via the image crate) as the decoded cover.
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 50, 50]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let path = std::path::PathBuf::from("cover-test.gblorb");
        let mut cover = app::cover::CoverState::default();
        cover.insert(path.to_path_buf(), app::cover::decode(&png));

        // Deterministic, terminal-free protocol.
        let picker = ratatui_image::picker::Picker::halfblocks();

        // Mirror draw_story_picker_full_width_then_split for cs + buffer setup.
        let cs = app::colors::ColorScheme::default();
        let area = Rect::new(0, 0, 40, 24);
        let mut buf = Buffer::empty(area);

        let meta = minimal_story_meta(); // helper defined below

        super::draw_info_panel(
            "Cover Test", "cover-test.gblorb", &meta, None,
            0, area, Some(&picker), &mut cover, &path, &path, false, None, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );

        // Half-blocks emit the upper-half-block glyph in the reserved top band.
        // Collect the columns holding image pixels.
        let band_rows = area.top()..area.top() + area.height / 2;
        let img_cols: Vec<u16> = (area.left()..area.right())
            .filter(|&x| {
                band_rows
                    .clone()
                    .any(|y| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}"))
            })
            .collect();
        assert!(!img_cols.is_empty(), "cover band should contain half-block pixels");

        // The fitted (square) cover is CENTERED within the band, not left-aligned:
        // there is letterbox margin on both sides. Panel border is at x=0, so the
        // band's inner content starts at x=1.
        let min_x = *img_cols.iter().min().unwrap();
        let max_x = *img_cols.iter().max().unwrap();
        assert!(min_x > 1, "cover should have a left letterbox margin (leftmost col = {min_x})");
        assert!(
            max_x < area.right() - 2,
            "cover should have a right letterbox margin (rightmost col = {max_x})"
        );

        // The band is now sized to the image's aspect-fitted height (`used_h`),
        // not a fixed half-panel box: the info text should begin immediately
        // under the image, with no dead letterbox rows pushing it down.
        let last_image_row = band_rows
            .clone()
            .filter(|&y| {
                (area.left()..area.right())
                    .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}"))
            })
            .max()
            .expect("cover band should contain at least one image row");
        let title_row = (area.top()..area.bottom())
            .find(|&y| {
                let row_text = (area.left()..area.right())
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>();
                row_text.contains("Cover Test")
            })
            .expect("title text should appear in the panel");
        assert_eq!(
            title_row,
            last_image_row + 1,
            "title should begin immediately under the fitted image, no dead letterbox rows \
             (last image row = {last_image_row}, title row = {title_row})"
        );
    }

    // ── Story-picker info panel: toggle/slide/split ─────────────────────────────

    #[test]
    fn slide_fraction_interpolates_and_reverses() {
        // A closed→open slide at t=0 is 0.0, at t=1 is 1.0; reversing mid-slide
        // starts from the current fraction.
        let mut s = super::PanelSlide::closed();
        assert_eq!(s.fraction_at(0.0), 0.0);
        s.toggle_to(true, /*instant=*/true);
        assert_eq!(s.fraction_at(1.0), 1.0);
        s.toggle_to(false, true);
        assert_eq!(s.fraction_at(1.0), 0.0);
    }

    #[test]
    fn panel_refuses_to_open_when_too_narrow() {
        // Below LIST_MIN_W + PANEL_MIN_W the toggle is a no-op.
        assert!(!super::can_open_panel(super::LIST_MIN_W + super::PANEL_MIN_W - 1));
        assert!(super::can_open_panel(super::LIST_MIN_W + super::PANEL_MIN_W));
    }

    #[test]
    fn draw_story_picker_full_width_then_split() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = make_two_test_stories();
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(2);

        // Closed: list uses full width, no panel border cell on the right edge.
        let area = Rect::new(0, 0, 70, 12);
        let mut buf = Buffer::empty(area);
        let (list_area, panel_area) = super::split_picker_area(area, 0.0);
        assert_eq!(list_area.width, area.width);
        assert_eq!(panel_area.width, 0);

        // Open (fraction 1.0): list shrinks, a panel area with width >= PANEL_MIN_W appears.
        let (list_area, panel_area) = super::split_picker_area(area, 1.0);
        assert!(list_area.width < area.width);
        assert!(panel_area.width >= super::PANEL_MIN_W);
        let _ = (&stories, &badges, &glyphs, &cs, &mut buf, &mut list);
    }

    // ── SQ-0348: fetch-progress wiring ──────────────────────────────────────────
    //
    // `run_story_picker` itself can't be unit-tested (it owns a real terminal),
    // so these exercise the pieces the loop wires together: `resort_list`
    // (the caches stay index-aligned with `stories`), the progress-line
    // overlay, and — the important one — a simulated `Fetcher` sweep driving
    // `resolve_entry` + `resort_preserving_selection` exactly as the loop's
    // drain handler does, proving the selection survives titles landing mid-sweep.

    /// Minimal valid v3 story bytes (mirrors `picker.rs`'s private test fixture
    /// of the same name — not reusable across modules, so duplicated here).
    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        buf[0x18] = 0x00; buf[0x19] = 0x60;
        // A printable serial (ZMSD §11.1, `$12`–`$17`) — required since SQ-0889,
        // when a Z-machine image started having to look like one.
        buf[0x12..0x18].copy_from_slice(b"000000");
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        app::scratch_dir(&format!("picker-ui-{tag}"))
    }

    #[test]
    fn resort_list_keeps_row_badges_and_aux_cache_aligned_with_the_new_order() {
        let stories_dir = temp_dir("resort-align");
        std::fs::write(stories_dir.join("a.z5"), minimal_v3_story()).unwrap();
        let mut b_bytes = minimal_v3_story();
        b_bytes[0x12] = b'9'; // distinct serial → distinct IFID from a.z5
        std::fs::write(stories_dir.join("b.z5"), b_bytes).unwrap();
        let data_base = temp_dir("resort-align-data");
        let hint_index = app::hints::load_hint_index(&data_base);

        let mut stories = app::picker::scan_stories(&stories_dir, &data_base);
        assert_eq!(stories.len(), 2);
        let mut row_badges: Vec<app::picker::RowBadges> = stories
            .iter()
            .map(|e| app::picker::compute_row_badges(e, &data_base, &hint_index))
            .collect();
        let mut aux_cache: Vec<Option<app::picker::StoryAux>> = vec![Some(app::picker::resolve_aux(
            &stories[0],
            &data_base,
            &hint_index,
        ))];
        aux_cache.push(None);
        let selected_path = stories[0].path.clone();

        let new_idx = super::resort_list(
            &mut stories,
            0,
            app::picker::Sort { key: app::picker::SortKey::Title, desc: true },
            &mut row_badges,
            &mut aux_cache,
            &data_base,
            &hint_index,
        );

        assert_eq!(stories[new_idx].path, selected_path, "selection follows its story");
        assert_eq!(row_badges.len(), stories.len(), "row_badges stays index-aligned");
        assert_eq!(aux_cache.len(), stories.len(), "aux_cache stays index-aligned");
        assert!(aux_cache.iter().all(Option::is_none), "a reorder invalidates every cached aux slot");

        let _ = std::fs::remove_dir_all(&stories_dir);
        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn progress_line_overlays_the_footer_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        // Pre-fill the footer row with something the overlay must fully replace,
        // proving it clears trailing characters rather than just prefixing.
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, area.bottom() - 1)) {
                c.set_symbol("#");
            }
        }
        let story_header_active = cs.theme.get("story_header_active").style;
        super::draw_progress_line(&mut buf, area, "Fetching 7/23 — Zork I", story_header_active);
        let row = row_text(&buf, area.bottom() - 1, area);
        assert!(row.contains("Fetching 7/23 — Zork I"), "{row:?}");
        assert!(!row.contains('#'), "the overlay must clear the whole row, not just prefix it: {row:?}");
        let cell = buf.cell((area.left(), area.bottom() - 1)).unwrap();
        assert_eq!(
            Some(cell.fg), story_header_active.fg,
            "progress line must use a themed style, not a hard-coded color"
        );
    }

    /// A `MetadataSource` fake local to this module (the one in
    /// `fetch_worker`'s tests is private to that module) — canned responses
    /// keyed by IFID, never touching the network.
    struct FakeSource {
        title_by_ifid: std::collections::HashMap<String, String>,
    }

    impl app::ifdb::MetadataSource for FakeSource {
        fn fetch(&self, ifid: &str) -> Result<app::ifdb::FetchOutcome, app::ifdb::FetchError> {
            match self.title_by_ifid.get(ifid) {
                Some(title) => Ok(app::ifdb::FetchOutcome::Found(Box::new(app::ifiction::IFiction {
                    title: Some(title.clone()),
                    ..Default::default()
                }))),
                None => Ok(app::ifdb::FetchOutcome::NotFound),
            }
        }
        fn fetch_by_id(&self, _tuid: &str) -> Result<app::ifdb::FetchOutcome, app::ifdb::FetchError> {
            Ok(app::ifdb::FetchOutcome::NotFound)
        }
        fn fetch_cover(&self, _url: &str) -> Result<Vec<u8>, app::ifdb::FetchError> {
            Ok(Vec::new())
        }
    }

    /// THE highest-value integration test in this task: drives a real
    /// `Fetcher` (with a fake source, zero delay) over two stories, then runs
    /// the exact same drain-handling pipeline the picker loop uses —
    /// `resolve_entry` to pick up the freshly-written sidecar, then
    /// `resort_preserving_selection` — and checks the selection followed its
    /// story through a title-driven reorder, not its index.
    #[test]
    fn a_simulated_sweep_lands_new_titles_and_the_selection_follows_its_story() {
        let stories_dir = temp_dir("sweep");
        // "zork2.z5" starts as a bare stem title that sorts LAST (after the
        // untouched "other.z5" control story); the sweep gives it a fetched
        // title that sorts FIRST, so a naive index-based cursor would end up
        // pointing at the wrong (unrelated) story once the sweep lands.
        std::fs::write(stories_dir.join("other.z5"), minimal_v3_story()).unwrap();
        let mut b_bytes = minimal_v3_story();
        b_bytes[0x12] = b'9';
        std::fs::write(stories_dir.join("zork2.z5"), b_bytes.clone()).unwrap();
        let data_base = temp_dir("sweep-data");

        let mut stories = app::picker::scan_stories(&stories_dir, &data_base);
        assert_eq!(stories.len(), 2);
        let selected = stories.iter().position(|e| e.path.ends_with("zork2.z5")).unwrap();
        let ifid_b = stories[selected].meta.ifid.clone();

        let mut title_by_ifid = std::collections::HashMap::new();
        title_by_ifid.insert(ifid_b, "AAA Brand New Title".to_string());
        let fetcher = app::fetch_worker::Fetcher::new(
            Box::new(FakeSource { title_by_ifid }),
            data_base.clone(),
            std::time::Duration::ZERO,
        );
        let order: Vec<app::fetch_worker::FetchTarget> =
            stories.iter().map(app::fetch_worker::FetchTarget::row).collect();
        fetcher.request(app::fetch_worker::FetchOrder { stories: order, forced: true, id_override: None });

        // Bounded drain (mirrors fetch_worker's own test pattern): collect
        // progress until both stories report in, or give up after ~2s.
        let mut progress = Vec::new();
        for _ in 0..2000 {
            progress.extend(fetcher.drain());
            if progress.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(progress.len(), 2, "both stories must report a completed fetch");

        // Exactly what the picker loop's drain handler does per progress item.
        for p in &progress {
            if let Some(fresh) = app::picker::resolve_entry(&p.path, &data_base) {
                if let Some(slot) = stories.iter_mut().find(|e| e.path == p.path) {
                    *slot = fresh;
                }
            }
        }
        assert_eq!(stories[selected].title, "AAA Brand New Title", "the sidecar write landed");

        let new_idx =
            app::picker::resort_preserving_selection(&mut stories, selected, app::picker::Sort::default());
        assert_eq!(new_idx, 0, "the new title now sorts first");
        assert!(stories[new_idx].path.ends_with("zork2.z5"), "selection followed its story, not its old index");
        assert_eq!(stories[new_idx].title, "AAA Brand New Title");

        let _ = std::fs::remove_dir_all(&stories_dir);
        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn gallery_centers_titles_for_missing_covers_and_frames_selection() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};
        let cs = app::colors::ColorScheme::terminal_default();
        let stories = vec![
            story_with_meta("Zork", None, None),
            story_with_meta("Anchorhead", None, None),
            story_with_meta("Curses", None, None),
        ];
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut tiles = app::cover::TileEncoder::detached();
        let mut first_row = 0usize;
        // No picker → no cover art → each tile shows its title centred in the band.
        let (rects, cols, vis) = super::draw_story_gallery(
            &stories, 1, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")), &cs, &km(), None, false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );

        assert!(cols >= 1 && vis >= 1);
        assert_eq!(rects.len(), 3, "one hit-rect per story tile");
        // Missing covers render the title centred in the cover band itself.
        let whole: String = (area.top()..area.bottom())
            .map(|y| row_text(&buf, y, area))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(whole.contains("Zork"), "missing-cover title: {whole:?}");
        assert!(whole.contains("Anchorhead"));
        assert!(whole.contains("Curses"));

        // The selected tile (index 1) is highlighted across its whole background
        // AND framed in the surrounding gutter; an unselected tile is neither.
        // SQ-0309: `story_tile_selected` now resolves through the theme (accent
        // role + reversed/bold) layered over the pane's dialog-chrome background
        // via `Cell::set_style`'s patch semantics (unset fields — here `bg` — are
        // left as whatever was painted underneath), so the highlight shows up as
        // REVERSED video rather than as an explicit `bg` colour; check for that
        // instead of comparing the raw `.bg` (or the whole `Style`, which would
        // also fold in whatever the cell happened to have underneath it).
        let sel_style = cs.theme.get("story_tile_selected").style;
        assert!(sel_style.add_modifier.contains(Modifier::REVERSED), "sanity: selected style is reversed video");
        let sel_tile = rects.iter().find(|(i, _)| *i == 1).unwrap().1;
        let interior = buf.cell((sel_tile.x, sel_tile.y)).unwrap();
        assert!(interior.style().add_modifier.contains(Modifier::REVERSED), "selected tile background is highlighted");
        assert_eq!(interior.fg, sel_style.fg.unwrap(), "selected tile uses the accent colour");
        assert_eq!(interior.symbol(), "┌", "missing-cover placeholder has a border");
        let frame_above = buf.cell((sel_tile.x, sel_tile.y - 1)).unwrap();
        assert!(frame_above.style().add_modifier.contains(Modifier::REVERSED), "selection frame sits in the gutter");
        // An unselected tile (index 0): neither its background nor its gutter is tinted.
        let unsel_tile = rects.iter().find(|(i, _)| *i == 0).unwrap().1;
        let unsel_interior = buf.cell((unsel_tile.x, unsel_tile.y)).unwrap();
        assert!(!unsel_interior.style().add_modifier.contains(Modifier::REVERSED), "unselected tile is not highlighted");
        let unsel_above = buf.cell((unsel_tile.x, unsel_tile.y - 1)).unwrap();
        assert!(!unsel_above.style().add_modifier.contains(Modifier::REVERSED), "unselected tile has no frame");
    }

    /// A 2×2 red PNG, encoded via the `image` crate (mirrors cover.rs fixtures).
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// A minimal blorb carrying one `Pict` resource (number 1) holding `png`.
    fn blorb_with_pict(png: &[u8]) -> Vec<u8> {
        fn iff_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // one entry
        ridx.extend_from_slice(b"Pict");
        ridx.extend_from_slice(&1u32.to_be_bytes()); // number
        let ridx_chunk_len = 8 + (4 + 12); // header + count + one 12-byte entry
        let pict_off = 12 + ridx_chunk_len; // FORM/IFRS header + RIdx chunk
        ridx.extend_from_slice(&(pict_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff_chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&iff_chunk(b"PNG ", png));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn info_panel_records_a_hit_rect_for_a_previewable_pict_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        use app::picker::{ChunkInfo, Engine, Features, StoryMeta};
        let cs = app::colors::ColorScheme::terminal_default();
        // A self-contained blorb story with one image resource.
        let meta = StoryMeta {
            size_bytes: 1, story_bytes: 1, modified: None, engine: Engine::ZCode, format: "Blorb".into(),
            version: None, serial: None, release: None, ifid: "X".into(),
            features: Features::default(),
            self_blorb: Some(vec![ChunkInfo {
                usage: "Pict".into(), number: 3, chunk_type: "PNG ".into(), len: 100, detail: None,
            }]),
            disk_image: None,
            disk_entry: None,
            author: None, year: None, genre: None, language: None, description: None,
            ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 40, 30);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("/tmp/game.gblorb");
        let mut resource_rects: Vec<(Rect, super::ResourceRef)> = Vec::new();
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, entry_path,
            false, None, &cs, &mut buf, &mut Vec::new(), &mut resource_rects,
        );
        assert_eq!(resource_rects.len(), 1, "the Pict row is clickable");
        let (_, rref) = &resource_rects[0];
        assert_eq!(rref.kind, super::PreviewKind::Image);
        assert_eq!(rref.number, 3);
        assert_eq!(rref.blorb_path, entry_path, "self-blorb resource reads from the story itself");
    }

    #[test]
    fn open_preview_decodes_an_image_resource_from_a_blorb() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("lanthorn-preview-{}.blb", std::process::id()));
        std::fs::write(&path, blorb_with_pict(&tiny_png())).unwrap();

        let rref = super::ResourceRef {
            blorb_path: path.clone(),
            kind: super::PreviewKind::Image,
            number: 1,
            label: "Image #1".into(),
        };
        let mut audio = None;
        let pv = super::open_resource_preview(&rref, &mut audio, 100);
        let _ = std::fs::remove_file(&path);
        assert!(pv.image.is_some(), "the Pict PNG decoded");
        assert!(pv.status.is_none(), "a decodable image needs no status line");
        assert!(audio.is_none(), "an image preview never touches the audio backend");
    }

    #[test]
    fn open_preview_of_a_missing_blorb_yields_a_status_not_a_panic() {
        let rref = super::ResourceRef {
            blorb_path: std::path::PathBuf::from("/no/such/file.blb"),
            kind: super::PreviewKind::Image,
            number: 1,
            label: "Image #1".into(),
        };
        let mut audio = None;
        let pv = super::open_resource_preview(&rref, &mut audio, 100);
        assert!(pv.image.is_none());
        assert!(pv.status.is_some(), "an unreadable blorb surfaces a status line");
    }

    // ── Image-preview zoom (SQ-0486) ────────────────────────────────────────

    #[test]
    fn open_resource_preview_starts_at_fit_zoom() {
        let rref = super::ResourceRef {
            blorb_path: std::path::PathBuf::from("/no/such/file.blb"),
            kind: super::PreviewKind::Image,
            number: 1,
            label: "Image #1".into(),
        };
        let mut audio = None;
        let pv = super::open_resource_preview(&rref, &mut audio, 100);
        assert_eq!(pv.zoom, super::PreviewZoom::Fit, "default on open is the fitted view");
    }

    #[test]
    fn zoom_step_in_from_fit_lands_on_native_size() {
        assert_eq!(super::PreviewZoom::Fit.step_in(), super::PreviewZoom::Factor(1));
    }

    #[test]
    fn zoom_step_in_increments_the_factor() {
        assert_eq!(super::PreviewZoom::Factor(1).step_in(), super::PreviewZoom::Factor(2));
        assert_eq!(super::PreviewZoom::Factor(5).step_in(), super::PreviewZoom::Factor(6));
    }

    #[test]
    fn zoom_step_in_clamps_at_the_max_factor() {
        let maxed = super::PreviewZoom::Factor(super::MAX_ZOOM_FACTOR);
        assert_eq!(maxed.step_in(), maxed, "already at the cap: another zoom-in is a no-op");
    }

    #[test]
    fn zoom_step_out_from_native_size_returns_to_fit() {
        assert_eq!(super::PreviewZoom::Factor(1).step_out(), super::PreviewZoom::Fit);
    }

    #[test]
    fn zoom_step_out_decrements_the_factor() {
        assert_eq!(super::PreviewZoom::Factor(3).step_out(), super::PreviewZoom::Factor(2));
    }

    #[test]
    fn zoom_step_out_at_fit_is_a_no_op() {
        assert_eq!(super::PreviewZoom::Fit.step_out(), super::PreviewZoom::Fit, "1× is the floor");
    }

    #[test]
    fn zoom_step_in_then_out_round_trips() {
        let z = super::PreviewZoom::Fit;
        assert_eq!(z.step_in().step_in().step_out(), super::PreviewZoom::Factor(1));
    }

    #[test]
    fn zoom_label_formats_fit_and_factor() {
        assert_eq!(super::PreviewZoom::Fit.label(), "Fit");
        assert_eq!(super::PreviewZoom::Factor(1).label(), "1\u{d7}");
        assert_eq!(super::PreviewZoom::Factor(4).label(), "4\u{d7}");
    }

    #[test]
    fn center_crop_rect_is_a_no_op_when_the_scaled_image_already_fits() {
        // 100x80 scaled image inside a roomier 200x150 budget: nothing to crop.
        let (x, y, w, h) = super::center_crop_rect((100, 80), (200, 150));
        assert_eq!((x, y, w, h), (0, 0, 100, 80));
    }

    #[test]
    fn center_crop_rect_clamps_to_the_budget_and_centres_the_crop() {
        // A 300x200 scaled image over a 100x50 budget crops to the budget size,
        // offset so the crop is centred (not anchored to a corner).
        let (x, y, w, h) = super::center_crop_rect((300, 200), (100, 50));
        assert_eq!((w, h), (100, 50));
        assert_eq!(x, (300 - 100) / 2);
        assert_eq!(y, (200 - 50) / 2);
    }

    #[test]
    fn center_crop_rect_only_clamps_the_overflowing_axis() {
        // Wide budget but short: width fits untouched, height gets cropped.
        let (x, y, w, h) = super::center_crop_rect((100, 200), (500, 50));
        assert_eq!((x, w), (0, 100), "width already fits: no horizontal crop");
        assert_eq!(h, 50);
        assert_eq!(y, (200 - 50) / 2);
    }

    #[test]
    fn gallery_scroll_follows_selection_offscreen() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        // Enough stories that a low selection is on a grid row below the fold.
        let stories: Vec<_> = (0..40).map(|i| story_with_meta(&format!("S{i}"), None, None)).collect();
        // Small area → few visible rows, so a late index forces a scroll.
        let area = Rect::new(0, 0, 40, 22);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut tiles = app::cover::TileEncoder::detached();
        let mut first_row = 0usize;
        let (rects, _cols, _vis) = super::draw_story_gallery(
            &stories, 39, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")), &cs, &km(), None, false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert!(first_row > 0, "grid scrolled down to keep the last cover visible");
        assert!(rects.iter().any(|(i, _)| *i == 39), "the selected tile is on screen");
    }

    // ── SQ-1213: gallery scroll-settle debounce ─────────────────────────────
    //
    // Same pattern as SQ-1198's `sixel_scroll_suppress` tests in
    // `render/inline_image.rs`: a pure gate test on the suppression DECISION,
    // plus a render OUTCOME test asserted on the buffer cells `draw_story_gallery`
    // writes. The picker runs its own standalone event loop with no `AppState`
    // to ride, so `gallery_scroll_in_motion`/`gallery_sixel_scroll_suppress`
    // track the identical 150ms window locally instead.

    /// `gallery_sixel_scroll_suppress` gates on BOTH the backend and the motion
    /// window: only sixel, and only while `gallery_scroll_in_motion` reads true.
    /// Kitty re-places an existing upload by id for free and half-blocks are
    /// ordinary cells, so neither pays the cost this debounce exists to avoid.
    #[test]
    fn gallery_scroll_suppress_gates_on_protocol_and_motion() {
        let mut sixel = app::render::graphics::kitty_picker(8, 16);
        sixel.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
        let kitty = app::render::graphics::kitty_picker(8, 16);
        let halfblocks = ratatui_image::picker::Picker::halfblocks();

        // Never scrolled this session: never suppressed, whatever the backend.
        assert!(!super::gallery_scroll_in_motion(None), "never scrolled = not in motion");
        assert!(!super::gallery_sixel_scroll_suppress(&sixel, false), "no motion yet");
        assert!(!super::gallery_sixel_scroll_suppress(&kitty, false));
        assert!(!super::gallery_sixel_scroll_suppress(&halfblocks, false));

        // Freshly scrolled: in motion, and sixel alone is suppressed.
        let fresh = Some(std::time::Instant::now());
        assert!(super::gallery_scroll_in_motion(fresh), "a fresh scroll is in motion");
        assert!(super::gallery_sixel_scroll_suppress(&sixel, true), "sixel mid-scroll must suppress");
        assert!(!super::gallery_sixel_scroll_suppress(&kitty, true), "kitty is untouched by the debounce");
        assert!(!super::gallery_sixel_scroll_suppress(&halfblocks, true), "half-blocks is untouched");

        // Past the settle window (backdating the Instant, since the test can't
        // literally sleep for it — mirrors the SQ-1198 state.rs tests).
        let stale = Some(std::time::Instant::now() - std::time::Duration::from_millis(200));
        assert!(!super::gallery_scroll_in_motion(stale), "past the settle window");
    }

    /// Falsification target: while suppressed, a sixel tile with a decoded
    /// cover renders as its letterbox footprint only — no protocol is built or
    /// placed, so no cell carries a sixel payload — where a settled render of
    /// the exact same tile places the real protocol. Repeated suppressed
    /// renders (simulating a burst of scroll steps, all still inside the
    /// window) place nothing at all; only the first settled render after the
    /// burst places the real payload.
    ///
    /// SQ-1199 added the second half of the claim: a suppressed frame does not
    /// even ASK for the raster. Encoding moved to a worker, so a fling that
    /// requested every tile it flew past would queue a row of encodes per notch
    /// for payloads no suppressed frame is going to place. The settled render
    /// requests, the worker answers, and the frame after that places — which is
    /// why this case now drives the encode to completion between the two.
    #[test]
    fn suppressed_gallery_render_shows_footprint_only_settled_places_once() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let story = story_with_meta("Zork", None, None);
        let stories = vec![story.clone()];
        let area = Rect::new(0, 0, 40, 22);
        let mut cover = app::cover::CoverState::default();
        // A real encoder: `drain_blocking` waits on the worker's own reply
        // rather than on a clock, so this stays deterministic.
        let mut tiles = app::cover::TileEncoder::new();
        // A real decoded cover, so there is something to place.
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 0, 0])));
        cover.insert(story.path.clone(), Some(img));

        let mut picker = app::render::graphics::kitty_picker(8, 16);
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Sixel);
        let cell = (picker.font_size().width, picker.font_size().height);

        // A cell carrying the real sixel payload is far longer than any glyph
        // or plain space this view otherwise paints.
        let has_payload = |buf: &Buffer| {
            (area.left()..area.right()).any(|x| {
                (area.top()..area.bottom())
                    .any(|y| buf.cell((x, y)).is_some_and(|c| c.symbol().len() > 16))
            })
        };

        let mut first_row = 0usize;
        // Three suppressed renders in a row (a burst of scroll steps, all still
        // inside the debounce window): none may place the real payload, and
        // none may queue an encode for one.
        for _ in 0..3 {
            let mut buf = Buffer::empty(area);
            super::draw_story_gallery(
                &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
                &cs, &km(), Some(&picker), true, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
            );
            assert!(!has_payload(&buf), "mid-scroll must not carry a sixel payload");
            assert!(!tiles.pending(), "mid-scroll must not queue an encode either");
        }

        // Settled: the render asks for the raster (and still places nothing).
        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert!(tiles.pending(), "the settled render queues the tile's encode");

        // The worker answers; the next render places the real protocol.
        for (key, proto) in tiles.drain_blocking() {
            cover.insert_tile(key, proto.expect("the tile encodes"), cell);
        }
        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert!(has_payload(&buf), "settled render must place the real sixel payload");
    }

    // ── SQ-1199: gallery tile protocols are encoded off the UI thread ───────
    //
    // The draw's job is now to ENQUEUE, not to encode. These cases drive
    // `draw_story_gallery` against a `TileEncoder::detached()` — a worker-less
    // encoder whose request channel the harness reads and whose replies the
    // harness writes — so "the draw did not encode", "it deduped", and "that
    // reply is stale" are all assertable without a thread or a clock.

    /// N visible tiles with decoded-but-unencoded covers: the draw places NO
    /// protocol and enqueues exactly N requests; an immediate redraw (the 16ms
    /// tick fires while they are in flight) enqueues none of them again; and
    /// once the replies are delivered the next draw places them.
    ///
    /// Falsification (dedupe half): drop the `in_flight` guard in
    /// `TileEncoder::request` and the second draw queues all N a second time —
    /// which, at a 16ms tick, is how a still-encoding grid queues the same work
    /// dozens of times over.
    #[test]
    fn gallery_draw_enqueues_tile_encodes_without_building_them() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let stories: Vec<_> =
            (0..4).map(|i| story_with_meta(&format!("S{i}"), None, None)).collect();
        let area = Rect::new(0, 0, 80, 40);
        let mut cover = app::cover::CoverState::default();
        let mut tiles = app::cover::TileEncoder::detached();
        for s in &stories {
            let img =
                image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([9, 200, 9])));
            cover.insert(s.path.clone(), Some(img));
        }
        let picker = app::render::graphics::kitty_picker(8, 16);
        let cell = (picker.font_size().width, picker.font_size().height);
        let has_payload = |buf: &Buffer| {
            (area.left()..area.right()).any(|x| {
                (area.top()..area.bottom())
                    .any(|y| buf.cell((x, y)).is_some_and(|c| c.symbol().len() > 16))
            })
        };

        let mut first_row = 0usize;
        let mut buf = Buffer::empty(area);
        let (rects, _, _) = super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert_eq!(rects.len(), stories.len(), "sanity: every tile is on screen");
        assert!(!has_payload(&buf), "the draw must not build a protocol on this thread");
        let queued = tiles.take_requests();
        assert_eq!(queued.len(), stories.len(), "one encode request per visible tile");

        // A redraw while they are all still in flight queues nothing new.
        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert!(tiles.take_requests().is_empty(), "in-flight tiles are not re-requested");

        // Deliver every reply, the way the picker loop's drain does.
        for r in queued {
            let proto = app::render::graphics::fitted_protocol(
                &r.picker,
                &r.img,
                ratatui::layout::Size::new(r.key.cols, r.key.rows),
                false,
            );
            tiles.deliver(r.key, proto);
        }
        for (key, proto) in tiles.drain() {
            cover.insert_tile(key, proto.expect("the tile encodes"), cell);
        }
        assert!(!tiles.pending(), "every request was answered");

        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        assert!(has_payload(&buf), "delivered tiles paint on the next draw");
    }

    /// The cell changed shape between the request and the reply (SQ-0988's
    /// font-size resize, which throws every built raster away): the reply is
    /// fitted to a cell the terminal no longer has, so it is DISCARDED rather
    /// than cached — and the redraw at the new cell asks for a fresh one under
    /// a key of its own.
    ///
    /// Falsification: drop the `key.cell != cell` guard in
    /// `CoverState::insert_tile` and the stale raster is kept — `insert_tile`
    /// returns true, and it sits in the LRU under a geometry nothing will ever
    /// look up again.
    #[test]
    fn a_tile_reply_for_a_stale_cell_is_discarded() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let story = story_with_meta("Zork", None, None);
        let stories = vec![story.clone()];
        let area = Rect::new(0, 0, 40, 22);
        let mut cover = app::cover::CoverState::default();
        let mut tiles = app::cover::TileEncoder::detached();
        let img =
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 0, 0])));
        cover.insert(story.path.clone(), Some(img));

        // Requested at an 8x16 cell.
        let picker = app::render::graphics::kitty_picker(8, 16);
        let mut first_row = 0usize;
        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&picker), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        let queued = tiles.take_requests();
        assert_eq!(queued.len(), 1, "one tile, one request");
        let req = queued.into_iter().next().unwrap();
        assert_eq!(req.key.cell, (8, 16), "the request carries the cell it was fitted for");
        let proto = app::render::graphics::fitted_protocol(
            &req.picker,
            &req.img,
            ratatui::layout::Size::new(req.key.cols, req.key.rows),
            false,
        )
        .expect("the tile encodes");

        // Meanwhile the font size moved: the picker (and `invalidate_cell_geometry`)
        // is now on a 10x20 cell, and the reply above is fitted to a cell that
        // no longer exists.
        cover.invalidate_cell_geometry();
        let wide = app::render::graphics::kitty_picker(10, 20);
        let cell = (wide.font_size().width, wide.font_size().height);
        assert_ne!(req.key.cell, cell, "sanity: the cell really did change");
        assert!(
            !cover.insert_tile(req.key.clone(), proto, cell),
            "a reply fitted to the old cell must be discarded, not cached"
        );
        assert!(cover.tile(&req.key).is_none(), "and it is not in the cache under its own key");

        // The redraw at the new cell asks again, under a key of its own.
        tiles.drain();
        let mut buf = Buffer::empty(area);
        super::draw_story_gallery(
            &stories, 0, &mut first_row, &super::PickerHeading::browse(std::path::Path::new("/tmp")),
            &cs, &km(), Some(&wide), false, &mut cover, &mut tiles, std::path::Path::new("/tmp"), area, &mut buf,
        );
        let again = tiles.take_requests();
        assert_eq!(again.len(), 1, "the new cell's tile is requested afresh");
        assert_eq!(again[0].key.cell, cell, "under the CURRENT cell's key");
    }
}
