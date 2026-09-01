//! Story-pane renderer over the engine-neutral [`ScreenModel`] tree.
//!
//! One renderer draws both engines. The **simple** case — a single text-grid
//! over a single text-buffer (the Z-machine shape), or a lone buffer — routes to
//! the existing `draw_upper_window` + `render_transcript` path, so the Z-machine
//! output stays byte-identical. Any richer Glulx tree (multiple/other windows)
//! uses the generic recursive path: `Pair` splits the rect and recurses, `Grid`
//! leaves draw positioned cells, the **primary** `Buffer` leaf draws through the
//! transcript renderer (keeping search / persistence / styling), extra buffers
//! draw their inline content, and `Blank`/graphics leaves are placeholders.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::colors::ColorScheme;
use crate::engine::{BorderPref, BufferWindow, Introspect, PositionedWindow, ScreenModel, StatusModel, WinKind, WinNode};
use crate::render::TextInk;
use crate::render::transcript::{draw_str_runs, render_transcript, visible_wrapped_lines_kinded};
use crate::render::upper_window::{draw_grid, draw_grid_transparent, draw_upper_window};
use crate::state::{AppState, TranscriptKind};

/// Metrics the story-pane render reports back for scrollbar / mouse routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryPaneMetrics {
    /// Whether the (primary) transcript drew a scrollbar gutter.
    pub scrollbar: bool,
    /// The largest meaningful `transcript_scroll` value.
    pub max_scroll: u16,
    /// The transcript viewport height: the rows of the pane that actually carry
    /// prose this frame, once the status line, the input bar, a suggestion strip
    /// and (while it is showing) the `[more]` row have taken theirs. It used to be
    /// the whole pane rect, which counted every one of those as readable and left
    /// the pager measuring against rows the reader never gets (SQ-0823).
    pub viewport_rows: u16,
    /// Rows the `[more]` prompt takes OUT of `viewport_rows` while it shows: `1`
    /// on the cell paths (it reserves its own row at the foot of the transcript),
    /// `0` on the raster path, which draws the prompt as an overlay on the last
    /// prose row and so gives up nothing. The pager parks the view on the frame
    /// BEFORE the prompt appears, so this is what tells it the screenful it is
    /// aiming at will be one row shorter (SQ-0823).
    pub prompt_rows: u16,
    /// Total wrapped rows of the transcript this frame (for the [more] pager,
    /// which needs the true total even when it fits — SQ-0404).
    pub total_rows: u16,
    /// Per-frame map from rendered cell `(col, row)` → Glk hyperlink value, for
    /// hit-testing a mouse click to its link. Empty when nothing is linked.
    pub links: Vec<((u16, u16), u32)>,
    /// Whether this frame actually laid the transcript out. `false` on frames
    /// whose story pane carries no text surface at all — a v6 full-screen
    /// picture (splash, Zork Zero's map/rebus takeovers). Cross-frame transcript
    /// bookkeeping (the scroll clamp, the [more] pager's row baseline) must skip
    /// such frames: measuring "rows added" against a picture frame's zero total
    /// re-paged the ENTIRE backlog when the normal frame returned (SQ-0578).
    pub transcript_surface: bool,
    /// Every Glk-identified leaf's ACTUAL drawn rect this frame, as `(win id,
    /// kind, absolute screen rect)`. gvm's own layout rect reserves a 1-cell
    /// border gutter per split whether or not the theme draws a rule there
    /// (`upper_window_border` defaults to `None`, SQ-0821), so a mouse/hyperlink
    /// hit-test against gvm's rect skews by every collapsed gutter between the
    /// pane origin and the window. Recording what was actually painted — "ask
    /// the drawing where it put the text" — is the fix (SQ-1203). Empty for the
    /// Z-machine/Scott simple path (no Glk ids to record).
    pub win_rects: Vec<(u32, WinKind, Rect)>,
}

/// Tally `(grids, buffers, others)` leaf windows in the tree. Used only by tests
/// now that [`is_simple`] classifies structurally (SQ-0325).
#[cfg(test)]
fn count_leaves(node: &WinNode) -> (u32, u32, u32) {
    match node {
        WinNode::Grid(_) => (1, 0, 0),
        WinNode::Buffer(_) => (0, 1, 0),
        WinNode::Blank => (0, 0, 1),
        // A Graphics leaf can't use the simple text path — counts as "other",
        // forcing the generic path.
        WinNode::Graphics(_) => (0, 0, 1),
        // A v6 layered composite (Phase 1b) is likewise never the simple text
        // shape — counts as "other". Its own items aren't tallied; nothing
        // reads this test-only helper's counts below leaf granularity.
        WinNode::Layered(_) => (0, 0, 1),
        WinNode::Pair { first, second, .. } => {
            let a = count_leaves(first);
            let b = count_leaves(second);
            (a.0 + b.0, a.1 + b.1, a.2 + b.2)
        }
    }
}

/// True only for the Z-machine shapes the simple grid/transcript path renders
/// byte-identically: a lone buffer or grid, or a grid status band strictly ABOVE
/// the buffer. Every real Glulx layout (nonzero `content_size` extent) renders
/// through the generic tree path instead, so borders and orientation are honoured
/// (SQ-0325). `content_size == (0, 0)` is the Z-machine marker (`session.rs`
/// hardcodes it; `AppGlk` sets a real extent), so a Glulx grid-over-buffer that
/// once matched here — e.g. Counterfeit Monkey — now correctly takes the generic
/// path. That path always draws `model.grid()` as a full-width top band over the
/// transcript, so it is only correct for the one Z-machine orientation.
fn is_simple(model: &ScreenModel) -> bool {
    if model.content_size != (0, 0) {
        return false;
    }
    match &model.root {
        WinNode::Buffer(_) | WinNode::Grid(_) => true,
        WinNode::Pair { vertical: true, first, second, .. } => {
            matches!(**first, WinNode::Grid(_)) && matches!(**second, WinNode::Buffer(_))
        }
        _ => false,
    }
}

/// The game's live input colour (fg/bg) for the input line, or None when
/// colours are off or the game left both channels Default (theme-neutral).
///
/// The pair comes from `ScreenModel.bg`/`fg` — the PANE PAGE — for v1–5, but a
/// v6 story has no pane page (see `session::v6_screen_model`): every window
/// carries its own pair (§8.3) and the model's stays `Default`. So v6 reads the
/// STORY WINDOW's explicit pair instead, the same source the page/ink already
/// use (`v6::story_bg_rgba`/`story_fg_rgba`) — otherwise the typed input falls
/// back to the theme's grey `input_text` on the game's own white page, while the
/// prose beside it (coloured per-run from its `TextAttrs`) is black. Cell-side,
/// so the packed colours resolve through `resolve_zcolour` exactly as the prose
/// runs in `draw_str_runs` do. (SQ-0532 wave-6)
///
/// **And under that, the MACHINE's own pair** (SQ-0847). A window that declares
/// nothing is not a window with no colours — on the two machines whose §8.3.3
/// defaults ARE the screen it is a window standing on the machine's page, which
/// `session::machine_screen_pair` publishes as the v6 model's `fg`/`bg` and
/// [`v6_machine_page`] already lays under the prose. The input line had no such
/// layer: it resolved `input_text` over the transcript style, and that selector
/// derives from the `text` role, whose shipped ink is **white**. On the
/// Macintosh's white page — release 296 off `stories/Zork Zero Disk.image`, which
/// never calls `set_colour` at all — that is white on white, and the player could
/// not see what he was typing until he pressed Enter and the game's own echo
/// re-drew it as prose in the machine's black.
///
/// The machine's pair is a DEFAULT, though, and a `style.toml` that names the
/// input line's colours outranks a default — hence [`input_line_is_themed`].
/// `set_colour` is the other case entirely and is untouched: there the story
/// asked for a colour, and an honoured game colour has always beaten the theme's
/// input fields.
fn game_input_style(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Style> {
    if !state.config.honor_game_colours {
        return None;
    }
    let (fg, bg) = match &model.root {
        WinNode::Layered(items) => {
            let story = crate::render::v6_layout::classify_windows(items, state.v6_text.cell()).story;
            let (f, b) = crate::render::v6_layout::story_pair_packed(story);
            let (f, b) = (crate::state::unpack_zcolour(f), crate::state::unpack_zcolour(b));
            let declared_none = matches!(f, zvm::screen::ZColour::Default)
                && matches!(b, zvm::screen::ZColour::Default);
            if declared_none && !input_line_is_themed(&state.colors) {
                (crate::state::unpack_zcolour(model.fg), crate::state::unpack_zcolour(model.bg))
            } else {
                (f, b)
            }
        }
        _ => (crate::state::unpack_zcolour(model.fg), crate::state::unpack_zcolour(model.bg)),
    };
    if matches!(fg, zvm::screen::ZColour::Default) && matches!(bg, zvm::screen::ZColour::Default) {
        return None;
    }
    let mut s = ratatui::style::Style::new();
    if !matches!(fg, zvm::screen::ZColour::Default) {
        s = s.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, zvm::screen::ZColour::Default) {
        s = s.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    Some(s)
}

/// Has the player named the input line's own ink in `style.toml`? (SQ-0847)
///
/// The two selectors the typed line and its `>` are drawn through. Registry
/// default means nobody claimed the channel, and an unclaimed channel is the
/// machine's to fill; anything above it — global `style.toml`, a discovered
/// `garglk.ini`, the per-game sidecar — is a choice, and a choice outranks a
/// default. Per-SELECTOR, matching the resolver's own stamp (`Provenance` is not
/// per-channel; see `theme::resolve`).
fn input_line_is_themed(colors: &ColorScheme) -> bool {
    ["input_text", "input_prompt"]
        .iter()
        .any(|sel| colors.theme.get(sel).provenance != crate::theme::resolve::Provenance::Default)
}

/// The colour scheme to draw the grid (upper/status) window with. When the game
/// has set a page colour scheme (so the story pane is painted with it), the grid's
/// base `upper_window` colour is overridden to those page colours, so a reverse-video
/// status line reverses the GAME's page (e.g. black-on-white → a white-on-black
/// status bar) instead of the app theme — keeping the status bar consistent with the
/// recoloured pane. Borrows the theme unchanged when no game scheme is set. (SQ-0262)
fn grid_scheme<'a>(state: &'a AppState, model: &ScreenModel) -> std::borrow::Cow<'a, ColorScheme> {
    use zvm::screen::ZColour;
    if !state.config.honor_game_colours {
        return std::borrow::Cow::Borrowed(&state.colors);
    }
    let fg = crate::state::unpack_zcolour(model.fg);
    let bg = crate::state::unpack_zcolour(model.bg);
    if matches!(fg, ZColour::Default) && matches!(bg, ZColour::Default) {
        return std::borrow::Cow::Borrowed(&state.colors);
    }
    let mut c = state.colors.clone();
    let mut base = c.theme.get("upper_window").style;
    if !matches!(fg, ZColour::Default) {
        base = base.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        base = base.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    // The border chrome is entirely our own presentation — Glk provides no border
    // styling — so paint the frame in the same page colours as the content, making
    // the whole status area (content + border) one coloured block on the recoloured
    // page rather than a themed frame around a game-coloured interior. (SQ-0267)
    let mut border = c.theme.get("upper_window_border").style;
    if !matches!(fg, ZColour::Default) {
        border = border.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        border = border.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    // SQ-0309: `draw_grid`/`draw_window_separator` read `upper_window` and
    // `upper_window_border` through `c.theme` (the legacy fields are gone), so
    // the override must land in the theme those selectors derive from (their
    // registry parents are the `chrome`/`line` roles with no delta of their
    // own, so seeding just those two roles reproduces `base`/`border` exactly).
    // Other role-derived selectors this Cow's theme could serve (e.g.
    // `hyperlink`, off `accent`) fall back to the terminal-default role rather
    // than the user's real one — narrow, since only the grid/separator draw path
    // reads this Cow's theme, and only while a game page colour is honoured.
    let mut roles = crate::theme::resolve::Roles::terminal_default();
    roles.chrome = base;
    roles.line = border;
    c.theme = crate::theme::resolve::resolve(&roles, &Default::default(), &Default::default(), &Default::default());
    std::borrow::Cow::Owned(c)
}

/// Render the engine's screen into the story-pane `area`, returning scrollbar /
/// scroll metrics for the (primary) transcript.
///
/// The frame is closed here (SQ-0756): a v6 mapping this pass recorded is handed to
/// [`AppState::note_v6_frame_end`], which keeps the game's own frames for
/// `/dump-windows`. It has to be the frame BOUNDARY rather than any one render path —
/// five of them write the mapping, each from a different arm — and it has to be after
/// the path has run, since the mapping is what is being kept.
pub fn render_story_pane(
    model: &ScreenModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> StoryPaneMetrics {
    let m = render_story_pane_frame(model, char_mode, introspect, state, area, buf);
    state.note_v6_frame_end();
    m
}

fn render_story_pane_frame(
    model: &ScreenModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> StoryPaneMetrics {
    // Per-frame: the v6 Layered arm republishes this if the game named a page
    // (SQ-0704). Cleared first so a non-v6 frame — or a v6 game that declares
    // nothing — can never inherit the last frame's page.
    state.v6_story_page.set(None);
    // SQ-0740: the MACHINE's own screen pair, for the v6 surfaces that resolve a
    // default ink and page (`v6_host_pair`, the chrome ring's run style). Only
    // §8.3's Amiga interpreter publishes one — `session::v6_screen_model` reads it
    // back out of the header — and only while colours are honoured, since a pair
    // the interpreter paints with is still a game colour. Cleared first, so a frame
    // that declares none can never inherit the last one's.
    state.v6_page_pair.set(
        (state.config.honor_game_colours
            && matches!(model.root, WinNode::Layered(_))
            && !matches!(crate::state::unpack_zcolour(model.bg), zvm::screen::ZColour::Default))
        .then_some((model.fg, model.bg)),
    );
    // Paint the story-pane background with the game's current background
    // (theme-safe: only the story pane, never the map/chrome; only a concrete,
    // honoured background — Default keeps the theme).
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        let page = if !matches!(bg, zvm::screen::ZColour::Default) {
            Some(crate::render::resolve_zcolour(bg, &state.colors))
        } else {
            // SQ-0873: …and where the game names nothing, the MACHINE's page, if
            // this launch earned one. That is the only source a v1–v4 story can
            // have: `$2C`/`$2D` are v5+ header bytes, so a v3 game reports
            // `Default` here however faithfully the medium named its machine, and
            // the flood above never fired for the games the period look is FOR.
            //
            // Without it the page reached only the cells that carry text —
            // `apply_to_theme` patches the transcript's style, and a style paints
            // where a glyph is drawn. Blank rows, the gap after a short line and
            // the space between paragraphs all stayed on the host theme, so the
            // screen came out striped instead of being the machine's screen.
            //
            // The game's own background still wins: this is the branch where it
            // declared none.
            state
                .period_look
                .map(|l| ratatui::style::Color::Rgb(l.page.0, l.page.1, l.page.2))
        };
        if let Some(bg_color) = page {
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(ratatui::style::Style::new().bg(bg_color));
                    }
                }
            }
        }
    }

    let gi = game_input_style(model, state);

    if is_simple(model) {
        // Byte-identical Z-machine path: the upper grid (if any) over the
        // transcript.
        let mut links: Vec<((u16, u16), u32)> = Vec::new();
        let gc = grid_scheme(state, model);
        let used = match model.grid() {
            Some(grid) => draw_upper_window(grid, char_mode, &gc, area, buf, state.config.honor_game_colours, &mut links),
            None => 0,
        };
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let tarea = reserve_text_margin(tarea, state, margin_style(model, state), buf);
        let mut t = render_transcript(&model.status, introspect, state, tarea, buf, gi);
        links.append(&mut t.links);
        // Unlike the multi-window path below, this one never opens a graphics
        // window — but an inline transcript image's eviction still queues a
        // delete here (SQ-1190), so it needs the same flush.
        state.graphics_render.borrow_mut().flush_kitty_deletes(area, buf);
        return StoryPaneMetrics {
            scrollbar: t.scrollbar,
            max_scroll: t.max_scroll,
            viewport_rows: t.viewport_rows,
            prompt_rows: t.prompt_rows,
            total_rows: t.total_rows,
            links,
            transcript_surface: true,
            win_rects: Vec::new(),
        };
    }

    // Generic multi-window path. Grid windows push their hyperlink cells into
    // `grid_links`; the primary buffer's own links ride on its metrics. (SQ-0258)
    //
    // Clamp the composite to gvm's content bounding box: gvm snaps proportional
    // splits to whole cells and leaves a blank margin, so walking the tree into the
    // FULL pane would let the last right-spine leaf balloon to absorb the surplus
    // width. Render into the box and keep the margin blank (SQ-0303).
    let inner = content_bounds(model, area);
    let mut grid_links: Vec<((u16, u16), u32)> = Vec::new();
    let mut win_rects: Vec<(u32, WinKind, Rect)> = Vec::new();
    let gc = grid_scheme(state, model);
    let metrics = render_node(&model.root, &model.status, char_mode, introspect, state, inner, buf, gi, &mut grid_links, &mut win_rects, &gc);
    // Keep gvm's snap-margin (the strips of `area` outside `inner`) clean, so no
    // stale cells from a prior frame or the map remain beside the window tree.
    fill_margin(area, inner, model, state, buf);

    // Prune the graphics protocol cache to only the windows still live in the
    // tree, so a closed window's stale cache entry can't be matched by a
    // reopened window reusing the same id (SQ-0174).
    let mut live = std::collections::HashSet::new();
    collect_graphics_ids(&model.root, &mut live);
    {
        let mut gr = state.graphics_render.borrow_mut();
        gr.retain_live(&live);
        // Closing the last graphics window leaves no placement to carry the deletes
        // its uploads need, so hand them a cell of this frame instead (SQ-0637).
        gr.flush_kitty_deletes(area, buf);
    }

    let mut m = metrics.unwrap_or(StoryPaneMetrics {
        scrollbar: false,
        max_scroll: 0,
        viewport_rows: area.height,
        prompt_rows: 0,
        total_rows: 0,
        links: Vec::new(),
        transcript_surface: false,
        win_rects: Vec::new(),
    });
    m.links.extend(grid_links);
    m.win_rects.extend(win_rects);
    m
}

/// The sub-rect of the story pane that gvm's window tree actually covers: the
/// top-left corner of `area` sized to `model.content_size`, clamped to `area`.
/// gvm snaps proportional splits to whole cells and leaves a blank margin
/// (SQ-0303); clamping the composite (and the graphics-rect walk, so
/// `dialog_bounds` agrees with what's drawn) to this keeps the margin blank
/// instead of ballooning the last right-spine window. Falls back to the full
/// `area` when `content_size` is `(0, 0)` (the simple/Z-machine paths — no margin).
pub fn content_bounds(model: &ScreenModel, area: Rect) -> Rect {
    // A v6 Layered root is PIXEL content: the raster/hybrid paths scale the
    // native game frame (e.g. Zork0's 320x200 ≈ 40x25 cells) up to fill the
    // pane, so clamping to the cell content_size would pin the whole game to
    // a native-size postage stamp in the corner (the SQ-0303 gvm snap-margin
    // clamp is for cell-fixed window trees only).
    if matches!(model.root, crate::engine::WinNode::Layered(_)) {
        return area;
    }
    let (cw, ch) = model.content_size;
    if cw == 0 || ch == 0 {
        return area;
    }
    Rect::new(area.x, area.y, cw.min(area.width), ch.min(area.height))
}

/// The background style gvm's snap-margin should be painted with: the game's
/// honoured page background when it set a concrete one (matching the story-pane
/// fill at the top of `render_story_pane`), else the theme transcript background
/// (matching `fill`).
fn margin_style(model: &ScreenModel, state: &AppState) -> ratatui::style::Style {
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        if !matches!(bg, zvm::screen::ZColour::Default) {
            return ratatui::style::Style::new().bg(crate::render::resolve_zcolour(bg, &state.colors));
        }
    }
    state.colors.theme.get("transcript").style
}

/// The text-window inner margin actually applied inside `area`, as
/// `(horizontal, vertical)` cells.
///
/// A discovered garglk.ini's `tmarginx`/`tmarginy` wins (highest precedence,
/// runtime-only — never persisted), else the global config default (SQ-0344);
/// either is capped so at least one cell of text survives. Shared by
/// [`reserve_text_margin`] (which inset the transcript by it) and
/// [`story_screen_dims`] (which must report the same width to the story).
fn effective_text_margin(area: Rect, state: &AppState) -> (u16, u16) {
    let ov = state.garglk_overlay.as_ref();
    let want_x = ov.and_then(|o| o.margin_x).unwrap_or(state.config.text_margin_x);
    let want_y = ov.and_then(|o| o.margin_y).unwrap_or(state.config.text_margin_y);
    (
        want_x.min(area.width.saturating_sub(1) / 2),
        want_y.min(area.height.saturating_sub(1) / 2),
    )
}

/// The screen size, in character cells, that the Z-machine should be told the
/// host has — `(rows, cols)` for header bytes $20/$21.
///
/// ZMSD §8.4: the interpreter "may change the exact dimensions whenever it likes
/// but must write the current height (in lines) and width (in characters) into
/// bytes $20 and $21 in the header." So this measures the REAL story pane
/// (`area` is the pane's content rect) instead of reporting a fixed guess.
///
/// The number reported is the region the game's own screen actually gets:
///
/// - the text margin (`text_margin_x`) is subtracted, because that is where the
///   transcript wraps — declaring a wider screen would make a game's centred
///   full-width form sit wider than the prose beside it, the exact mismatch this
///   replaces;
/// - the transcript's one-column scrollbar gutter is subtracted for the same
///   reason (`render_transcript` always reserves it);
/// - the upper window's frame is subtracted, because `draw_grid` draws the grid
///   INSIDE that frame. Without this the declared width would not fit and the
///   game's rightmost columns would be clipped.
///
/// The three together are exactly the chrome that separates the pane from the
/// story's own columns, so the declared width IS the width the upper grid is
/// rendered at AND the width the transcript wraps at — they can no longer drift
/// apart the way a fixed 80 did.
///
/// `virtual_screen_cols`/`virtual_screen_rows` still win when the user pinned
/// them (see the config docs). Returns `None` for a zero-area pane (before the
/// first frame, or while the story pane is hidden) — there is nothing to report.
pub fn story_screen_dims(area: Rect, state: &AppState) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let sides = state.colors.upper_window_border_sides;
    let on = |s: crate::render::paneframe::BorderStyle| {
        u16::from(s != crate::render::paneframe::BorderStyle::None)
    };
    let border_cols = on(sides.left) + on(sides.right);
    let border_rows = on(sides.top) + on(sides.bottom);
    let (mx, _) = effective_text_margin(area, state);
    let gutter = u16::from(area.width >= 2);
    let cols = state
        .config
        .virtual_screen_cols
        .unwrap_or_else(|| (area.width - 2 * mx).saturating_sub(border_cols + gutter));
    let rows = state
        .config
        .virtual_screen_rows
        .unwrap_or_else(|| area.height.saturating_sub(border_rows));
    Some((rows.max(1), cols.max(1)))
}

/// The screen size actually DECLARED to a running story: [`story_screen_dims`]
/// for the pane, floored at the width the story booted believing it had.
///
/// [`story_screen_dims`] measures the pane, and for the *height* that is the
/// whole story — a game re-declares its upper window's height on every layout
/// (`split_window`, ZMSD §8.7.2.1), so it always re-reads the screen it is given.
///
/// The WIDTH it never re-declares: byte $21 is ours alone, and a v4/v5 status
/// routine reads it ONCE, when it lays the bar out, then updates the fields in
/// place at the column numbers it computed back then. Zork 1 (r52) is the
/// reference case — it paints the reverse-video bar at boot and thereafter only
/// `set_cursor`s to the two field columns it derived from the boot width. Narrow
/// the screen under it and those columns fall outside the window, where
/// §8.7.2.3 makes the move illegal; the interpreter drops it, and the digits
/// land wherever the cursor already was — column 1, on top of the room name.
/// That is the garbled status bar of SQ-0679, and no amount of care in the
/// renderer can undo it: by the time we draw, the game has already overwritten
/// its own text.
///
/// So the declared width may GROW to follow a widened pane (SQ-0533 —
/// Sherlock/Trinity, which do re-read $21, gain the columns; every coordinate
/// computed at the old width is still inside the new screen) but never SHRINK
/// below `boot_cols`, the width THIS session actually booted at. In a pane too
/// narrow for that, the story keeps painting the bar it was laid out for and
/// the pane clips the right of it — the same thing every terminal interpreter
/// shows in an 80-column game squeezed into a 60-column window, and a great
/// deal better than a bar with its room name eaten.
///
/// `boot_cols` is [`GameSession::boot_screen_cols`](crate::session::GameSession::boot_screen_cols):
/// [`zvm::screen::DEFAULT_SCREEN_COLS`] (80) for a session booted without a
/// pre-boot pane seed (SQ-0679's original assumption — every v4+ story used to
/// boot at the fixed default), or the real seeded column count (SQ-0680) when
/// one was given. Flooring at a fixed 80 regardless of what the session
/// actually booted at would silently overwrite a correctly narrow pre-boot
/// seed back up to 80 on the very next poll.
///
/// Exempt: v1–3 (no such header fields — §8.4 starts at v4), v6 (its screen is
/// the native pixel frame, scaled into the pane, never measured from it), and a
/// user-pinned `virtual_screen_cols` (explicit intent wins over our floor).
pub fn declared_story_screen_dims(
    area: Rect,
    state: &AppState,
    version: u8,
    boot_cols: u16,
) -> Option<(u16, u16)> {
    let (rows, cols) = story_screen_dims(area, state)?;
    if version < 4 || version == 6 || state.config.virtual_screen_cols.is_some() {
        return Some((rows, cols));
    }
    Some((rows, cols.max(boot_cols)))
}

/// Reserve the configured text-window inner margin (SQ-0345) inside a
/// text-buffer rect: paint the whole rect with `fill` so the reserved band reads
/// as clean padding, then return the inset rect the transcript draws into.
/// `text_margin_x` blank columns are reserved on each side and `text_margin_y`
/// blank rows top and bottom; a margin wider/taller than the rect is capped so at
/// least one cell of text survives. Applies to the text buffer only — the
/// text-grid/upper window is never inset (its cells are game-positioned). Because
/// `render_transcript` publishes its geometry from the rect it receives, insetting
/// here also keeps mouse selection and the copy path aligned (SQ-0197/SQ-0420).
fn reserve_text_margin(area: Rect, state: &AppState, fill: ratatui::style::Style, buf: &mut Buffer) -> Rect {
    let (mx, my) = effective_text_margin(area, state);
    // Publish the applied horizontal margin so the transcript draws its scrollbar
    // flush against the border (in the right margin band) rather than inset with
    // the text (SQ-0345). Set even in the no-op case so a stale value never leaks.
    state.text_margin_applied.set(mx);
    if mx == 0 && my == 0 {
        return area;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(fill);
            }
        }
    }
    Rect::new(area.x + mx, area.y + my, area.width - 2 * mx, area.height - 2 * my)
}

/// Blank gvm's snap-margin — the L-shaped region of `area` outside `inner` (the
/// full-height strip right of `inner`, plus the strip below `inner` within its
/// columns) — so no stale cells remain beside the clamped window tree (SQ-0303).
fn fill_margin(area: Rect, inner: Rect, model: &ScreenModel, state: &AppState, buf: &mut Buffer) {
    let style = margin_style(model, state);
    let paint = |r: Rect, buf: &mut Buffer| {
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    };
    let right = Rect::new(inner.right(), area.y, area.right().saturating_sub(inner.right()), area.height);
    let bottom = Rect::new(area.x, inner.bottom(), inner.width, area.bottom().saturating_sub(inner.bottom()));
    paint(right, buf);
    paint(bottom, buf);
}

/// Recursively render a tree node into `area`. Returns the primary buffer's
/// metrics when this subtree contains it. Grid-window hyperlink cells are pushed
/// into `links` (the primary buffer's own links ride on its returned metrics).
fn render_node(
    node: &WinNode,
    status: &StatusModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    game_input: Option<ratatui::style::Style>,
    links: &mut Vec<((u16, u16), u32)>,
    win_rects: &mut Vec<(u32, WinKind, Rect)>,
    grid_colors: &ColorScheme,
) -> Option<StoryPaneMetrics> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    match node {
        WinNode::Pair { vertical, split, border, key_bg, key_fg, first, second } => {
            // `*border` is the game's VETO (`winmethod_NoBorder`); the THEME decides
            // whether a rule is drawn at all, and a rule the theme never asked for
            // reserves no gutter either — no line, no gap (SQ-0821).
            let sep_style = border.then(|| separator_style(*vertical, grid_colors)).flatten();
            let (a1, sep, a2) =
                split_area_bordered(area, *vertical, split.fixed, u16::from(sep_style.is_some()));
            let m1 = render_node(first, status, char_mode, introspect, state, a1, buf, game_input, links, win_rects, grid_colors);
            // Only rule between two VISIBLE siblings. A border before a collapsed
            // (zero-extent) window — e.g. Counterfeit Monkey's image pane before it
            // shows a letter — would otherwise draw a stray rule with nothing beyond
            // it (SQ-0325). And skip our separator entirely when the game draws its
            // OWN divider as a graphics window adjacent to the gutter (Kerkerkruip),
            // so we don't double the line — matching a pixel interpreter that leaves
            // the border to the game's chrome (SQ-0332). Both SUPPRESS the rule
            // without giving the gutter back, which is what they did before SQ-0821:
            // the theme did ask for a border here, so its space stays reserved and no
            // layout shifts under a window that merely collapsed.
            let game_divider = edge_touches_painted_graphics(first, *vertical, true)
                || edge_touches_painted_graphics(second, *vertical, false);
            let draw = !a1.is_empty() && !a2.is_empty() && !game_divider;
            if let Some(bs) = sep_style.filter(|_| draw) {
                // The game's key-window colours only speak when the player has asked
                // for the game's colours at all (SQ-0821).
                let (kf, kb) =
                    if state.config.honor_game_colours { (*key_fg, *key_bg) } else { (None, None) };
                draw_window_separator(sep, *vertical, bs, kf, kb, grid_colors, buf);
            }
            let m2 = render_node(second, status, char_mode, introspect, state, a2, buf, game_input, links, win_rects, grid_colors);
            m1.or(m2)
        }
        WinNode::Grid(g) => {
            let show_cursor = char_mode && g.cursor_active;
            // Game-managed multi-window (generic) path: the game owns the layout
            // and draws its own borders (e.g. Kerkerkruip renders its panel rules
            // as graphics windows), so draw the grid FRAMELESS at its exact rect —
            // no app frame over the game's own separators, and no borrowed rows
            // (SQ-0303). The simple status-line path keeps the app frame via
            // `draw_upper_window`, so the Z-machine / Counterfeit Monkey status
            // bar (SQ-0267) is unaffected.
            let mut frameless = g.clone();
            frameless.border = BorderPref::NoBorder;
            draw_grid(&frameless, frameless.active_rows, frameless.cursor, show_cursor, grid_colors, area, buf, state.config.honor_game_colours, links);
            // Record the rect this grid was ACTUALLY drawn at (SQ-1203): a
            // Glk-identified grid's own click/hyperlink hit-test uses this, not
            // gvm's layout rect, which reserves a border gutter the theme may
            // draw thinner (or not at all, SQ-0821).
            if g.win != 0 {
                win_rects.push((g.win, WinKind::Grid, area));
            }
            None
        }
        WinNode::Buffer(b) => {
            if b.primary {
                let area = reserve_text_margin(area, state, state.colors.theme.get("transcript").style, buf);
                if b.win != 0 {
                    win_rects.push((b.win, WinKind::Buffer, area));
                }
                let t = render_transcript(status, introspect, state, area, buf, game_input);
                Some(StoryPaneMetrics {
                    scrollbar: t.scrollbar,
                    max_scroll: t.max_scroll,
                    viewport_rows: t.viewport_rows,
                    prompt_rows: t.prompt_rows,
                    total_rows: t.total_rows,
                    links: t.links,
                    transcript_surface: true,
                    win_rects: Vec::new(),
                })
            } else {
                render_inline_buffer(b, state, area, buf);
                if b.win != 0 {
                    win_rects.push((b.win, WinKind::Buffer, area));
                }
                None
            }
        }
        WinNode::Blank => {
            fill(area, buf, &state.colors);
            None
        }
        WinNode::Graphics(gw) => {
            // Solid/thin graphics windows (a game's chrome: panel dividers, colour
            // bars, backgrounds) render directly as cell backgrounds — exact,
            // grid-aligned, and legible even without an image protocol. A detailed
            // canvas falls through to the image protocol (or a plain fill). (SQ-0332)
            if crate::render::graphics::render_graphics_as_cells(gw, area, buf, false) {
                // painted as cells
            } else if let Some(picker) = state.game_picker.as_ref() {
                state.graphics_render.borrow_mut().render(picker, gw, area, state.colors.theme.get("graphics").style, buf);
            } else {
                // No image protocol: approximate the detailed canvas as colour
                // cells rather than blanking it (SQ-0520).
                crate::render::graphics::render_graphics_as_cells(gw, area, buf, true);
            }
            if gw.win != 0 {
                win_rects.push((gw.win, WinKind::Graphics, area));
            }
            None
        }
        WinNode::Layered(items) => {
            // Phase 1c: with an image protocol, composite the whole v6 pane as one
            // RGBA canvas in the game's NATIVE pixel space (graphics at exact
            // pixel coords, all text rasterized), then draw it scaled to fill the
            // pane. Without a picker, fall through to the Phase 1b cell composite.
            // With an OVERLAY open, likewise fall through: image placements draw
            // above terminal cells in classic protocols, so a menu/dialog under
            // the v6 image would be invisible — the cell fallback keeps the pane
            // readable behind the overlay until it closes.
            state.v6_image_scale.set(1.0);
            // Not the hybrid ring until the ring path below says so (SQ-1002).
            state.v6_hybrid_ring.set(false);
            // A painted MENU screen prints chrome text INSIDE the story window's
            // box, below the status band — Shogun's boot menu paints rows 21–23
            // over its story buffer (rows 21–25). In HYBRID mode such a takeover
            // screen must NOT take the pixel chrome ring: the ring path splits the
            // menu across the raster ring (items mapping above the terminal
            // viewport) and the terminal overlay (items inside it), the exact
            // mixed raster/text defect (SQ-0484). Routing it to the cell path
            // below renders it as one coherent all-text screen.
            // BOTH conditions matter (SQ-0494): a grid run that is
            // merely deep but sits OUTSIDE the story box is ordinary gameplay
            // chrome — Arthur paints its status bar at row 12 above a story
            // buffer starting at row 13, and classing that as a menu dropped
            // Arthur's whole ring (top image panel + side bars). RASTER mode
            // deliberately keeps its pixel composite for menus (the reverse-video
            // selection block is fixed in `build_chrome_canvas` instead,
            // SQ-0487) — a raster-mode user wants the pixel aesthetic even
            // on menus.
            let hybrid = state.config.v6_render == crate::config::V6RenderMode::Hybrid;
            let story_box = items.iter().find_map(|pw| {
                // The PRIMARY buffer only: a v6 game can publish a second, non-primary
                // prose window (SQ-0585), and taking its rows as the story box made an
                // ordinary split look like a menu takeover.
                matches!(&pw.node, WinNode::Buffer(b) if b.primary).then(|| {
                    let top = state.v6_text.cell().row_of_origin0(pw.y_px);
                    (top, top + pw.h_px.max(1).div_ceil(16), pw.x_px as u32, pw.x_px as u32 + pw.w_px as u32)
                })
            });
            let has_menu = items.iter().any(|pw| {
                matches!(&pw.node, WinNode::Grid(g)
                    if g.px_texts.iter().any(|t| {
                        let row = state.v6_text.cell().row_of(t.y);
                        // SQ-0742: the run must be inside the story box on BOTH axes. The
                        // row test alone calls any chrome glyph that merely shares a row
                        // with the story a takeover — and a game whose frame is drawn with
                        // LINE-DRAWING characters rather than reverse-video spaces has one
                        // on every row of the box. Journey under the Amiga profile draws
                        // exactly that: `│` rules at native x 0 / 256 / 632, all outside
                        // its story box (264..632), on every one of its rows. That routed a
                        // perfectly ordinary gameplay screen to the cell path, which draws
                        // the game's 80 columns 1:1 into a pane of any width — the frame
                        // stopped short of the pane edge and the click map (proportional
                        // over the whole pane) no longer matched where anything was drawn.
                        // Under the IBM PC profile the same rules are reverse-video SPACES,
                        // which trim to empty and never tripped the gate, which is why only
                        // the Amiga route showed it.
                        let x0 = t.x.max(1) as u32 - 1;
                        let x1 = x0 + state.v6_text.cell().run_px(&t.text).max(u32::from(state.v6_text.cell().w()));
                        !t.text.trim().is_empty()
                            && row >= STATUS_BAND_ROWS
                            && story_box.is_some_and(|(top, bot, left, right)| {
                                row >= top && row < bot && x1 > left && x0 < right
                            })
                    }))
            });
            // SQ-0886: …but the cell path DRAWS NO ART, so it is the wrong
            // destination for a takeover screen the game framed with artwork.
            // Shogun's boot menu is exactly that: its credits and its three items
            // sit on the machine's own ground between two ornate side panels, and
            // routing the screen to cells discarded both — no panels anywhere, and
            // the story window's page flooded across the pane (measured on
            // `James Clavell's Shogun.adf` release 295 and on the Blorb release 322
            // alike: `#000000` across 761 of 800 columns where the Amiga's colour 12
            // ground belongs).
            //
            // SQ-0892 RE-POINTED WHERE IT SENDS THEM. It sent them to the COMPOSITE,
            // because the ring could not lay this screen out — the reason recorded
            // below at the hybrid branch, and true until now: the ring drew the menu
            // one CHARACTER per independently rounded cell (`SI(RT th e ga me`). Both
            // halves of that are gone. SQ-0894 built the ring from content, so the
            // ornaments are one flank down the whole pane on either press; SQ-0892
            // groups a row's runs before placing them, so the menu is intact. The
            // frame now takes the RING, which draws the panels as art and the credits
            // and menu as CRISP GLYPHS — SQ-0750's rule, which the composite cannot
            // honour because it rasterises every character on the screen.
            //
            // What this predicate still decides, and why it is kept: a menu takeover
            // with NO art goes to the coherent all-text cell path (SQ-0484), and one
            // WITH art must not, because the cell path draws no art. That is the
            // distinction it was always making. Only its destination changed.
            //
            // ART, specifically — a chrome GRAPHICS window with opaque pixels in it.
            // An `erase_window` fill is not art: the cell path draws those itself
            // (`draw_erase_fills`), which is what keeps advent's boot popup — a
            // painted panel over a story with no artwork in the game at all — on the
            // coherent all-text path SQ-0484 put it on.
            let menu_over_art = has_menu
                && hybrid
                && items.iter().any(|pw| {
                    matches!(&pw.node, WinNode::Graphics(g)
                        if g.win != 0 && g.canvas.pixels().any(|p| p[3] >= 128))
                });
            // MODAL overlays only (SQ-0587). The fall-through exists because image
            // placements draw above terminal cells in classic protocols, so a
            // menu/dialog over the story pane would be invisible under the v6 image.
            // The room panel and the tidy animation are not that: both live in the MAP
            // pane and never cover the story, and the code already draws exactly this
            // distinction for the input line. Including them here meant an ordinary
            // move — which re-tidies the map and starts its animation — dropped the
            // whole v6 pixel path for the duration, and Arthur's header art vanished
            // with it.
            if !state.any_modal_overlay_open() && !(has_menu && hybrid && !menu_over_art) {
            if let Some(picker) = state.game_picker.as_ref() {
                let (default_fg, default_bg) = v6_host_pair(state);
                use crate::render::v6_layout as v6;
                let native = v6::native_extent(items, &state.v6_text);
                let layout = v6::classify_windows(items, state.v6_text.cell());
                // The native chrome canvas is built per-branch below (SQ-0469):
                // the raster arm skips the build entirely on an unchanged frame.

                // Hybrid mode (Lane H): draw the chrome as a scaled pixel RING
                // around a terminal story viewport, then render the story window as
                // real terminal text (crisp, selectable, scrollable) inside it — the
                // existing primary-Buffer transcript path, with inline images as
                // bands. Needs a story window; without one — or with a full-screen
                // picture takeover, which has no ring to draw (SQ-0570) — fall
                // through to raster.
                // SQ-0886 excluded a menu takeover over the game's own artwork here, so
                // it fell to the composite below; SQ-0892 removed that exclusion. The
                // two defects it named are both fixed: the ring no longer rasterises
                // the frozen banner into a full-width art band (SQ-0894 classifies a
                // band by its CONTENT, so a banner of text is a text strip), and it no
                // longer lays the menu out one CHARACTER per rounded cell (a row's
                // abutting runs are grouped before they are placed). Shogun's boot menu
                // now draws its panels as art and its credits and menu as glyphs, which
                // is what SQ-0750 asks for and what the composite structurally cannot
                // do — it rasterises every character on the screen.
                if state.config.v6_render == crate::config::V6RenderMode::Hybrid {
                    // WHICH arm decided this frame's route, published for
                    // `/dump-terminal` (SQ-0994). The routing test already computes
                    // it and used to throw it away, so a `Cell` store costs the
                    // frame path nothing — and the command cannot recompute it,
                    // because by the time a command runs the live frame is the
                    // palette's. `None` with no story window at all: that is a
                    // fall-through the hatch never got to judge.
                    let takeover = layout
                        .story
                        .and_then(|s| picture_takeover_reason(s, &layout.chrome, layout.story_gfx, native));
                    state.v6_takeover_reason.set(takeover);
                    if let Some(story) = layout.story.filter(|_| takeover.is_none()) {
                        // The op log's frame boundary (SQ-0590) opens HERE, not at
                        // the band draw further down: the two calls below are the
                        // frame's first protocol traffic, and starting the log after
                        // them erased the very records a lifecycle audit needs — the
                        // full-frame composite this frame drops, and the bands a
                        // resume re-uploads (SQ-0747).
                        state.graphics_render.borrow_mut().begin_band_log();
                        // This frame renders as chrome bands, not the raster
                        // composite — drop the cached composite so a later
                        // fall-through to raster (map/rebus takeover) cannot
                        // flash a stale screen while its encode runs (SQ-0578).
                        state.graphics_render.borrow_mut().invalidate_v6();
                        // Resuming the pixel path after ANY frame that did not use it
                        // (an overlay was up, a menu takeover, a raster frame): the
                        // terminal no longer holds our placements, but every band is a
                        // cache hit and would send nothing. Force them all to re-upload
                        // on this frame (SQ-0587).
                        if state
                            .v6_path_log
                            .borrow()
                            .last()
                            .is_none_or(|(label, _)| label != "hybrid-ring")
                        {
                            state.graphics_render.borrow_mut().invalidate_chrome_bands();
                        }
                        // SQ-0532 wave-5: a game that set its own story page presents
                        // on a FULL page — Zork Zero boots `set_colour(fg=2 black,
                        // bg=9 white)` and the DOS original's white runs edge to edge:
                        // behind the frame art, through the chrome band surrounds, out
                        // into the letterbox margins. Flood the whole pane with it
                        // before the ring and viewport draw over it (the ring's clear
                        // pixels then show the page, not the theme backdrop). Strictly
                        // gated on the story window's EXPLICIT bg, so a game that sets
                        // none — Journey's black picture panel, Arthur, Shogun's
                        // gameplay screen — keeps today's theme backdrop. Gated on
                        // the LIVE `honor_game_colours` too: the model keeps the
                        // pair the game recorded while colours were honored, so a
                        // `/set-game-colours off` mid-game must skip the flood
                        // here, not rely on the window reading `Default`.
                        if state.config.honor_game_colours {
                            if let Some(p) = v6::story_bg_rgba(Some(story), &state.colors) {
                                fill_pane_page(area, p, buf);
                                // Publish it for inline story pictures: their alpha is
                                // ours to resolve, against THIS page (SQ-0704).
                                state.v6_story_page.set(Some((p[0], p[1], p[2])));
                            }
                        }
                        // SQ-1187: THE HYBRID FRAME GATE. Everything the ring derives —
                        // the two native canvases, the layout plans, the strip carving,
                        // the flank probes — is a pure function of inputs `v6_hybrid_gen`
                        // folds into one key. On a match the cached `HybridFrame` is
                        // replayed: no canvas is rebuilt, no scan runs, and the band
                        // draws below reuse their stored content hashes (see
                        // `set_band_replay`). This is the raster arm's SQ-0469 gate,
                        // finally built for the shipped default mode: the compute half
                        // cost 3-8 ms per redraw on a 640x400 press and was paid per
                        // keystroke and per frame of any animation.
                        let hkey = v6_hybrid_gen(items, state, area, picker, story);
                        let cached = state.graphics_render.borrow_mut().hybrid.take().filter(|f| f.key == hkey);
                        let replayed = cached.is_some();
                        let frame = match cached {
                            Some(f) => f,
                            None => {
                                state.graphics_render.borrow_mut().hybrid_builds += 1;
                                build_hybrid_frame(hkey, &layout, story, native, area, picker, default_fg, default_bg, state)
                            }
                        };
                        let fs = picker.font_size();
                        let cell_px = (fs.width, fs.height);
                        let canvas = &frame.canvas;
                        let gfx = &frame.gfx;
                        let scale = frame.scale;
                        let menu = frame.menu;
                        let viewport = frame.viewport;
                        let vp_native = frame.vp_native;
                        let strip_has_art = |r: &Rect| frame.art_backed.contains(&(r.x, r.y, r.width, r.height));
                        let art_tiles = |role: v6::BandRole, r: Rect| -> Vec<Rect> {
                            if role.is_flank() { vec![r] } else { band_tiles(r, frame.tile_cols) }
                        };
                        let base = v6_machine_page(state, state.colors.theme.get("upper_window").style);
                        state.v6_scale_lock_inapplicable.set(frame.lock_inapplicable);
                        state.v6_scale_lock_fallback.set(frame.lock_fallback);
                        state.v6_image_scale.set(frame.image_scale);
                        state.v6_hybrid_ring.set(true);
                        state.v6_ring_plan.set(frame.ring_plan);
                        state.v6_ring_clip.set(frame.ring_clip);
                        {
                            let mut gr = state.graphics_render.borrow_mut();
                            // SQ-1187: tell the band draws whether this frame replays an
                            // unchanged HybridFrame — their content hashes are then read
                            // from the cache instead of recomputed. `begin_band_log`
                            // (above) already reset it for the frame.
                            gr.set_band_replay(replayed);
                            // SQ-0944: on a backend that resolves an image's alpha to
                            // BLACK, the ring's bands stop shipping alpha and resolve it
                            // here instead, onto the same page the raster composite has
                            // flattened onto since SQ-0510 — so half-blocks shows the
                            // story's own page in the gaps the frame art leaves, exactly
                            // as kitty does, rather than the encoder's black.
                            //
                            // Set on `gr` rather than carried into the three band entry
                            // points because the answer cannot differ between two bands of
                            // one frame; `begin_band_log` clears it, so it cannot outlive
                            // this one. Set HERE, after `clear_text_columns`, because the
                            // carves that punch holes in this canvas must all have run:
                            // flattening earlier would be undone by the next carve, and
                            // the flanks' own `v6_border::recognize` reads the canvas's
                            // TRANSPARENCY to tell a pillar from its ground — which is
                            // why the flatten lands on each band's finished image and
                            // never on the canvas they are all cut from.
                            gr.set_band_ground(
                                backend_flattens_alpha_to_black(picker)
                                    .then(|| v6_composite_page(layout.story, default_bg, state)),
                            );
                            gr.retain_chrome_bands(&frame.live);
                            for strip in &frame.strips {
                                if let ChromeStrip::Art(_, r) = strip {
                                    if !strip_has_art(r) {
                                        continue;
                                    }
                                    // SQ-0755: a flank whose rect an extension also covers is
                                    // drawn TWICE, to one cache key — two encodes a frame for
                                    // one visible image. Skipping the flank draw looked free and
                                    // is not: the extension replicates ONE native row, while the
                                    // flank band carries the column's whole native extent, and
                                    // dropping it measurably lost ink (one row, at a 138x68 pane
                                    // under halfblocks). The waste is real but it is an upload,
                                    // not a pixel; the pixels are not ours to throw away for it.
                                }
                                match strip {
                                    // Three ways an ART strip reaches the screen, and since
                                    // SQ-0898 all three draw at the frame's ONE magnification:
                                    // a Menu-plan flank PANEL (a picture fitted to a panel,
                                    // the one deliberate exception), a recognised side border
                                    // TILED to the rows the band asks for, and otherwise a
                                    // plain CROP of the shared scaled canvas.
                                    //
                                    // A fourth used to sit between the last two — SQ-0511's
                                    // Frame-plan vertical STRETCH — and it is gone; see the
                                    // note on the tiled arm below.
                                    ChromeStrip::Art(role, r) => {
                                        // SQ-0547: a Menu-plan SIDE flank is a panel — flood the
                                        // whole column with the game's own panel colour (sampled
                                        // from the art's outer edge) and centre the art in it,
                                        // instead of top-anchoring the art over bare backdrop.
                                        // The divider extension below re-draws over this fill, and
                                        // the frame bands stay wherever their own strips put them.
                                        // Resolved above so the dest rect could join the live set.
                                        let panel = frame.flank_panels.iter().find(|(sr, _)| sr == r).map(|(_, p)| *p);
                                        if let Some((bg, fill, dest, crop)) = panel {
                                            fill_pane_page(fill, bg, buf);
                                            gr.draw_chrome_band_stretched(
                                                picker,
                                                canvas,
                                                dest,
                                                crop,
                                                crate::render::graphics::BandSlot::Art,
                                                crate::render::graphics::BandFit::MenuPanel,
                                                buf,
                                            );
                                        } else if let Some((img, dest)) = frame.tiled_flanks.iter().find(|(sr, _)| sr == r).map(|(_, i)| i) {
                                            // SQ-0698: a recognised side border, tiled to
                                            // the band's own height at the uniform scale —
                                            // and placed at the device rect its native box
                                            // maps to, so the magnification IS the uniform
                                            // scale rather than whatever a fit to the band
                                            // would have produced (SQ-0898).
                                            gr.draw_chrome_band_image(picker, img, *r, *dest, crate::render::graphics::BandSlot::Art, buf);
                                        // SQ-0511's Frame-plan flank STRETCH stood here, and
                                        // SQ-0898 REMOVED IT with the measurement SQ-0894 asked
                                        // for and could not supply. That lane recorded the arm
                                        // as unreachable — disabling it passed the full gate —
                                        // and kept it on the reasoning that a green gate over a
                                        // corpus with no fixture proves nothing. The corpus did
                                        // have one; nothing drove it far enough to see it.
                                        //
                                        // It is reached by a flank the arm above DECLINES, and
                                        // `flank_source` declines for two reasons, not one.
                                        // "Unrecognised" is the reason SQ-0894 looked for and
                                        // no shipped title has. The other is `desired <= art.1`
                                        // — the band lies wholly INSIDE the artwork and needs
                                        // no extension at all — and Arthur reaches it on every
                                        // pane swept from 0.80 to 2.00, on both presses, for
                                        // the one turn his story window grows to the screen
                                        // bottom and selects `BottomPlan::Frame`. His 72-run
                                        // status bar cuts the pole in two and the piece above
                                        // it is entirely within the art.
                                        //
                                        // What it did there: crop the flank columns from the
                                        // band's top down to the ART's bottom — 387 native rows
                                        // — and stretch that into the band's 234 device px.
                                        // Vertical magnification 0.60 against the frame's 1.35,
                                        // measured at the user's 108x50 pane, so the whole pole
                                        // appeared squashed into the banner's height in each
                                        // top corner while the pole below it drew correctly.
                                        // §5 of the pipeline document predicted exactly this:
                                        // it stretches into the band's device box with no
                                        // aspect constraint at all.
                                        //
                                        // There is no correct use left to keep. The arm can
                                        // only fire where extension is impossible (nothing
                                        // recognised to extend) or unnecessary (the art already
                                        // covers the band); in the first case a stretch invents
                                        // rows by distorting the ones there are, and in the
                                        // second it distorts rows that were already complete.
                                        // Both fall to the plain crop below, which is the
                                        // frame's own magnification by construction.
                                        } else {
                                            // SQ-0818: the plain crop, one image per TILE.
                                            // `art_tiles` is the identity on a side flank,
                                            // and on every backend that does not tile.
                                            for t in art_tiles(*role, *r) {
                                                gr.draw_chrome_band(picker, canvas, &scale, area, t, buf);
                                            }
                                        }
                                    }
                                    ChromeStrip::Text(r, runs) => {
                                        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
                                        draw_chrome_text_strip(
                                            &refs, *r, &scale, cell_px, area, native, base, TextInk::of(state), buf,
                                            state.v6_text.cell(),
                                        )
                                    }
                                }
                            }
                            // SQ-0511 fix: extend each Menu flank's divider/border column
                            // through the reclaimed gap to the menu (a uniform column, so
                            // the vertical replicate is invisible); the rest of the gap is
                            // left undrawn (theme backdrop, matching the flank's own
                            // never-painted background beside the divider).
                            // SQ-0750: a border the game printed as a CHARACTER is stamped
                            // as that character, in the game's own style and colours —
                            // never uploaded as a bitmap of itself. This is the same
                            // column, in the same cells, that the band path drew; only the
                            // medium changes, so it now matches the frame's top and bottom
                            // rules (font glyphs both) instead of standing beside them as a
                            // resampled image of the same `│`.
                            for (ext, ink) in &frame.divider_exts {
                                match ink {
                                    BorderInk::Band(crop) => gr.draw_chrome_band_stretched(
                                        picker,
                                        canvas,
                                        *ext,
                                        *crop,
                                        crate::render::graphics::BandSlot::DividerExtension,
                                        crate::render::graphics::BandFit::DividerExtension,
                                        buf,
                                    ),
                                    // SQ-0779: the character stands in ONE column — its own,
                                    // `col` — and the rest of the extension is the native text
                                    // cell's own blank padding, drawn in the same style. Where a
                                    // terminal column is about one native cell those are the same
                                    // rect and nothing changes; where the scale is larger (2.93 at
                                    // the reported 236x68 terminal) the padding columns are the
                                    // ones the picture's band used to stand in, carrying a
                                    // rasterised copy of this very rule. Stamping the glyph across
                                    // all of them instead would be SQ-0750's doubled rule.
                                    BorderInk::Glyph { ch, style, fg, bg, col, .. } => {
                                        let st = v6_run_style(base, *fg, *bg, *style, TextInk::of(state));
                                        let g = ch.to_string();
                                        for y in ext.y..ext.bottom() {
                                            for x in ext.x..ext.right() {
                                                buf.set_stringn(x, y, if x == *col { &g } else { " " }, 1, st);
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(ms) = &menu {
                                for strip in &frame.menu_strips {
                                    match strip {
                                        ChromeStrip::Art(_, r) => gr.draw_chrome_band(picker, canvas, ms, area, *r, buf),
                                        ChromeStrip::Text(r, runs) => {
                                            let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
                                            draw_chrome_text_strip(
                                                &refs, *r, ms, cell_px, area, native, base, TextInk::of(state), buf,
                                                state.v6_text.cell(),
                                            )
                                        }
                                    }
                                }
                            }
                            // SQ-0944: the text the game printed ON its artwork, as
                            // GLYPHS over the bands just drawn. Last, so nothing the
                            // ring paints afterwards covers it again, and after the
                            // bands rather than before because the ground each glyph
                            // sits on is read back out of the cells they wrote.
                            //
                            // `over_art_runs` is empty unless the backend can layer a
                            // glyph over art, so on kitty, sixel and iTerm2 this is a
                            // walk over nothing.
                            if !frame.over_art_runs.is_empty() {
                                let over_art_refs: Vec<&crate::engine::PxText> =
                                    frame.over_art_runs.iter().collect();
                                let art_rects: Vec<Rect> = frame
                                    .strips
                                    .iter()
                                    .filter_map(|s| match s {
                                        ChromeStrip::Art(_, r) => Some(*r),
                                        ChromeStrip::Text(..) => None,
                                    })
                                    .collect();
                                stamp_runs_over_art(
                                    &over_art_refs, &art_rects, &scale, cell_px, area, gfx,
                                    default_fg, default_bg, &state.colors, buf,
                                    state.v6_text.cell(),
                                );
                            }
                            // Record the letterbox geometry for click→game-pixel mapping
                            // (Lane M). The regions themselves (`frame.packed_text`) are part
                            // of the cached hybrid frame — see `build_hybrid_frame`, where the
                            // full derivation (and its history) now lives.
                            let click_scale =
                                if frame.plan_is_menu { menu.as_ref().unwrap_or(&scale) } else { &scale };
                            gr.record_hybrid_click_map(area, click_scale, native, cell_px, frame.packed_text.clone());
                            // SQ-1188: hand this frame's changed-band encodes to the
                            // background worker in one batch. Every band above kept
                            // its old upload placed; the results land via
                            // `poll_v6_job` and the next redraw places them.
                            gr.spawn_band_jobs(picker);
                        }
                        // The story window as real terminal text (primary-Buffer path).
                        //
                        // …unless there is no story region at all: a plate that leaves no
                        // prose box owns the screen, and rasterizing the scrollback onto it
                        // would paint the PREVIOUS screen's text across the art (SQ-0707,
                        // which raster learned the hard way). Report no metrics, exactly as
                        // raster does on the same frame, so the scrollbar and the [more]
                        // machinery agree that nothing is showing. SQ-0896.
                        let metrics = if viewport.width == 0 || viewport.height == 0 {
                            None
                        } else if let WinNode::Grid(g) = &story.node {
                            // **A Grid in the story slot is drawn where the ring PUT it,
                            // and only where the game wrote** (SQ-1074/SQ-1075). This is
                            // SQ-1026's rule on the hybrid side: the InvisiClues screen
                            // publishes no buffer at all, so `classify_windows` resolves
                            // window 0 to a Grid, and the line below is the primary-Buffer
                            // path — `render_node`'s Grid arm then handed it to
                            // `draw_grid`, which is the GLULX/v3-v5 renderer and does
                            // three things that are right there and wrong here:
                            //
                            //   * it sizes the region as `grid.cols` TERMINAL columns,
                            //     when a v6 grid's columns are 8px NATIVE cells;
                            //   * it CENTRES that region in the area, when a v6 window
                            //     has an absolute native origin;
                            //   * it floods the region with the theme's `upper_window`
                            //     page, when a v6 window's ground is the game's.
                            //
                            // Amiga Shogun r295/890321 at a 115x34 pane is the report.
                            // Window 0 is 500x330 at native (70,70) — a 62x20 grid — and
                            // the ring gives it an 89x28 viewport at (13,6). Centring 62
                            // in 89 put the topic list at column 26, thirteen columns
                            // right of the window's own left edge, and the flood showed
                            // through the nine rows the game had not written since the
                            // clue screen: a 62x9 black rectangle, `Rgb(0, 0, 12)`, the
                            // theme's grid page. `machine-screenshots/amiga-shogun-hint.png`
                            // settles both halves — calibrated at native = image - 37, its
                            // topics start at image x=108, i.e. **native 71**, flush with
                            // the window's left edge, and the page runs the window's whole
                            // height with nothing black anywhere in it.
                            //
                            // `draw_grid_transparent` is the renderer that states all
                            // three correctly: 1:1 into the rect it is given, no centring,
                            // and only the cells the game actually wrote.
                            let show_cursor = char_mode && g.cursor_active;
                            draw_grid_transparent(g, viewport, buf, state.config.honor_game_colours, grid_colors, links, show_cursor);
                            // No metrics, exactly as `render_node`'s Grid arm reported
                            // before: there is no transcript on this frame.
                            None
                        } else {
                            render_node(&story.node, status, char_mode, introspect, state, viewport, buf, game_input, links, win_rects, grid_colors)
                        };
                        // SQ-0584: a chrome window the game ERASED more recently than it
                        // printed prose is an opaque panel over the story — advent.z6's
                        // `help` splits window 1 to 160px, erases it and paints a menu
                        // there. Fill its rect over the transcript before the runs below
                        // stamp their glyphs, so the panel reads as a panel instead of
                        // text floating over the room description. `fill` is only set
                        // for a window that is still the newest paint on its own rect,
                        // so an ordinary turn (whose prose is newer) fills nothing.
                        // Record what this frame mapped each window onto, in cells, for
                        // `/dump-windows`. Each entry carries the NATIVE rect it came
                        // from, so the engine can report a window's game-side state and
                        // its terminal placement as one block instead of leaving them to
                        // be correlated by eye (SQ-0585).
                        {
                            use crate::state::V6CellRect;
                            let rec = |label: &str, native: (u16, u16, u16, u16), r: Rect| V6CellRect {
                                label: label.to_string(),
                                native,
                                cells: (r.x, r.y, r.width, r.height),
                            };
                            let mut map = state.v6_cell_map.borrow_mut();
                            map.clear();
                            map.push(rec("path:hybrid-ring", (0, 0, 0, 0), area));
                            state.note_v6_path("hybrid-ring");
                            map.push(rec("pane", (0, 0, 0, 0), area));
                            // SQ-0896: the NATIVE rect beside the viewport is the one it
                            // was actually cut from — the window reduced to what the art
                            // leaves it. The declared window box is on the `story` line
                            // below, so a frame where the two differ says so in the dump
                            // instead of leaving it to be deduced.
                            map.push(rec(
                                "viewport",
                                (vp_native.0 as u16, vp_native.1 as u16, vp_native.2 as u16, vp_native.3 as u16),
                                viewport,
                            ));
                            map.push(V6CellRect {
                                label: "scale".into(),
                                native: ((scale.s * 100.0) as u16, scale.off_y as u16, cell_px.0, cell_px.1),
                                cells: (0, 0, 0, 0),
                            });
                            for pw in &layout.chrome {
                                let r = px_rect_to_cells(pw, &scale, cell_px, area, 0);
                                // SQ-0747: only the primary Buffer is DRAWN at the rect
                                // beside it here — the story viewport, as terminal cells.
                                // Every chrome Grid/Graphics window is rasterised into the
                                // chrome canvas and reaches the screen only through the
                                // strips listed below, so its line is where the window MAPS
                                // to, not a draw. Two investigations read a chrome grid's
                                // full-canvas mapping as a second paint over the top border's
                                // text strip — the two share row 1 by construction — and
                                // chased an overlap that does not exist. Say which is which.
                                let kind = match &pw.node {
                                    WinNode::Buffer(b) if b.primary => "story",
                                    WinNode::Buffer(_) => "panel",
                                    WinNode::Grid(_) => "grid (rasterised into the ring)",
                                    WinNode::Graphics(_) => "art (rasterised into the ring)",
                                    _ => "?",
                                };
                                map.push(rec(kind, (pw.x_px, pw.y_px, pw.w_px, pw.h_px), r));
                            }
                            for strip in &frame.strips {
                                match strip {
                                    ChromeStrip::Art(_, r) => {
                                        let label = if strip_has_art(r) {
                                            "strip:art".to_string()
                                        } else {
                                            "strip:art (skipped — no art behind it)".to_string()
                                        };
                                        map.push(rec(&label, (0, 0, 0, 0), *r))
                                    }
                                    ChromeStrip::Text(r, runs) => {
                                        map.push(rec(&format!("strip:text({} runs)", runs.len()), (0, 0, 0, 0), *r))
                                    }
                                }
                            }
                            // The bottom-anchored menu band's own strips (SQ-0742). They
                            // are classified through a DIFFERENT scale from the ring's and
                            // were absent from this dump entirely, so the rows between the
                            // menu and the pane bottom could not be accounted for at all.
                            for strip in &frame.menu_strips {
                                match strip {
                                    ChromeStrip::Art(_, r) => map.push(rec("menu:art", (0, 0, 0, 0), *r)),
                                    ChromeStrip::Text(r, runs) => {
                                        map.push(rec(&format!("menu:text({} runs)", runs.len()), (0, 0, 0, 0), *r))
                                    }
                                }
                            }
                            // The flank border columns carried down the reclaimed gap to the
                            // menu (SQ-0742). Their absence is invisible in every other line
                            // of this dump — the flank band is still there, still the right
                            // height, and simply has no ink below the game's own canvas.
                            // The native crop rides along (SQ-0750): the crop is resized to
                            // fill the band, so crop width vs band width IS the extension's
                            // horizontal magnification — and it has to be the letterbox
                            // scale, like everything else in the ring. Nothing else in the
                            // dump can show that it is not.
                            // The inner rule and the OUTER border are listed apart: they
                            // are the two edges the panel is bounded by, and "the outer
                            // one is missing" is a sentence this dump could not say
                            // before (SQ-0758).
                            for (_, inner, outer) in &frame.flank_borders {
                                for (label, e) in
                                    [("flank-divider", inner), ("flank-border", outer)]
                                {
                                    // …and a GLYPH border says so, and says which character
                                    // it stamps (SQ-0750). "The frame's sides are a bitmap
                                    // of a character" is the other sentence this dump could
                                    // not say, and the whole of that quest.
                                    match e {
                                        Some((ext, BorderInk::Band(crop))) => {
                                            let c = (crop.0 as u16, crop.1 as u16, crop.2 as u16, crop.3 as u16);
                                            map.push(rec(label, c, *ext));
                                        }
                                        Some((ext, BorderInk::Glyph { ch, style, col, native, .. })) => map.push(rec(
                                            // …and WHICH column of the extension the character
                                            // itself stands in, with the native text cell the
                                            // extension covers (SQ-0779). "The rule is one glyph
                                            // in a three-column cell" is the sentence that tells
                                            // a stamped border apart from a doubled one.
                                            &format!("{label} (glyph {ch:?} style={style:02b} at col {col})"),
                                            (native.0 as u16, 0, (native.1 - native.0) as u16, 0),
                                            *ext,
                                        )),
                                        None => {}
                                    }
                                }
                            }
                            // …and the panel FILL, which is the band clipped to the
                            // panel's own extent rather than the band itself (SQ-0747).
                            for (_, (_, fill, dest, _)) in &frame.flank_panels {
                                map.push(rec("flank-panel", (0, 0, 0, 0), *fill));
                                map.push(rec("flank-art", (0, 0, 0, 0), *dest));
                            }
                        }
                        // Only windows that START inside the story viewport fill here.
                        // Everything above it belongs to the chrome ring, which draws
                        // its own background — a status strip is flooded by its Text
                        // strip. Letting such a window fill too painted it twice, and
                        // the second rect is the PIXEL-scaled one: advent's 20px bar is
                        // 1.6 terminal rows at a tall pane, so its fill spilled a second
                        // row into the story and the bar read two rows deep (SQ-0582).
                        let fill_chrome: Vec<&PositionedWindow> = layout
                            .chrome
                            .iter()
                            .copied()
                            .filter(|pw| px_rect_to_cells(pw, &scale, cell_px, area, 0).y >= viewport.y)
                            .collect();
                        draw_erase_fills(
                            &fill_chrome, viewport, buf, base, TextInk::of(state),
                            &|pw: &PositionedWindow| px_rect_to_cells(pw, &scale, cell_px, area, 0),
                        );
                        draw_secondary_buffers(&layout.chrome, area, buf, state, &|pw: &PositionedWindow| {
                            px_rect_to_cells(pw, &scale, cell_px, area, 0)
                        });
                        // Chrome text runs that fall INSIDE the story box paint
                        // ON TOP of the terminal transcript (v6 paint order —
                        // Shogun overlays its boot-menu items and selection
                        // caret on the story strip; the ring canvas can't show
                        // them because the terminal viewport covers that area).
                        // Native px → device px (chrome-ring scale) → terminal
                        // cell, glyphs only (no background fill).
                        //
                        // SQ-0892: the runs of one native text row are grouped
                        // before they are placed, exactly as a chrome text STRIP
                        // has grouped its own since SQ-0509. A run is POSITIONED
                        // through the ring scale but then ADVANCES ONE TERMINAL
                        // COLUMN per character, and the two rates coincide only
                        // where a terminal column is one native 8px text cell — so
                        // rounding each fragment on its own scatters text the game
                        // printed as one stream. Shogun prints its boot menu glyph
                        // by glyph through a 1px caret window: fifteen abutting
                        // single-character runs at exactly 8px pitch, which at a
                        // 100x40 pane (1.225 columns per native cell) rounded to
                        // 36,37,38,40,41,… — neighbours colliding into one column
                        // and skipping the next, the `SI(RT th e ga me` this path
                        // was blamed for. Merged, the group is placed ONCE and its
                        // characters advance together, so it is intact and off by
                        // at most the quarter cell its origin was never on.
                        //
                        // [`merge_strip_fragments`] is that rule and is reused
                        // rather than restated: abutting fragments and one-cell
                        // word gaps join, a FIELD gap keeps its own column
                        // (SQ-0757). Nothing here is a rule or a divider — those
                        // are frame geometry, and frame geometry is not inside the
                        // story box — so the strip path's `collapse_row_rules`
                        // wrapper has nothing to do on this route.
                        let mut in_box: std::collections::BTreeMap<u16, Vec<&crate::engine::PxText>> =
                            std::collections::BTreeMap::new();
                        for it in &layout.chrome {
                            // SQ-0934: the promoted story grid is in `chrome` (its runs
                            // must reach `chrome_runs`) AND is the story surface, so
                            // `render_node` above has already drawn it into the viewport.
                            // Stamping it here as well draws every glyph twice — the
                            // rasterised-looking text under the glyphs that was reported.
                            if layout.story.is_some_and(|st| std::ptr::eq(*it, st)) {
                                continue;
                            }
                            if let WinNode::Grid(g) = &it.node {
                                for t in &g.px_texts {
                                    let px = t.x.max(1) as f32 - 1.0;
                                    let py = t.y.max(1) as f32 - 1.0;
                                    // SQ-0896: the boundary is the rect the VIEWPORT was
                                    // cut from, not the declared window box. They are the
                                    // same rectangle on every corpus frame; where the art
                                    // has moved the viewport, a run in the gap between them
                                    // is on the ring's side and the ring draws it — stamping
                                    // it here as well would draw it twice.
                                    if px < vp_native.0 as f32
                                        || px >= (vp_native.0 + vp_native.2) as f32
                                        || py < vp_native.1 as f32
                                        || py >= (vp_native.1 + vp_native.3) as f32
                                    {
                                        continue; // outside the story region → already in the ring
                                    }
                                    in_box.entry(t.y).or_default().push(t);
                                }
                            }
                        }
                        for row_runs in in_box.values_mut() {
                            row_runs.sort_by_key(|t| t.x);
                            // SQ-0937: the run's COLUMN is counted from the story box's
                            // own left edge — one terminal column per native 8px text
                            // cell — exactly as the row a few lines below is counted from
                            // its top. This is SQ-0892's rule, finally applied to the
                            // other axis.
                            //
                            // It used to map through the ring scale absolutely:
                            //
                            //     area.x + ((scale.off_x + px * scale.s) / cw).round()
                            //
                            // which disagrees with how the run is then DRAWN. `run_col`'s
                            // neighbours already say why: a run is positioned through the
                            // scale but "its characters then advance ONE TERMINAL COLUMN
                            // each, and the two rates coincide only where a column is one
                            // native 8px text cell." Positioning a run at one rate and
                            // advancing its characters at another makes the run drift away
                            // from its own text, and rounds its ORIGIN independently of the
                            // viewport it is being clipped against.
                            //
                            // MEASURED, on the Macintosh press (r296/881019) at a 136x50
                            // pane: the InvisiClues topic list prints its left column at
                            // native x=87 against a story box whose left edge is x=86. The
                            // old expression rounded that to `viewport.x - 1`, and the
                            // guard below DROPS a run outside the viewport — so the whole
                            // left column of the menu vanished while the right column at
                            // native x=320, far enough in to survive the rounding, drew
                            // normally. The same boundary the other way is the "disconnected
                            // reversed cells after the menu items" seen at other sizes.
                            //
                            // Counting from the box cannot disagree with the box: a run at
                            // the box's first column IS the viewport's first column, by
                            // construction. `px >= vp_native.0` is guaranteed by the filter
                            // that built `in_box`, so the subtraction never goes negative.
                            //
                            // SQ-1009: both terms are CELLS. The run's is the one the
                            // engine wrote it in, never its pixel origin divided by the
                            // declared width — that division is the column only while
                            // the pen advances one declared cell per character, and on a
                            // proportional machine it skips cells and drifts along the
                            // line. For a fixed pen it is the same subtraction it always
                            // was.
                            let vp_col = (vp_native.0 / u32::from(state.v6_text.cell().w())) as i32;
                            let run_col = |t: &crate::engine::PxText| {
                                viewport.x as i32 + i32::from(t.gcol) - vp_col
                            };
                            let merged = merge_strip_fragments(row_runs);
                            // SQ-0898: the cells this row's GLYPH runs occupy, so a
                            // BLANK run cannot erase one.
                            //
                            // This is SQ-0727's rule, at the one text site that never
                            // got it — reused rather than restated, exactly as SQ-0892
                            // reused `merge_strip_fragments` here. A run is POSITIONED
                            // through the ring scale but its characters then advance ONE
                            // TERMINAL COLUMN each, and the two rates coincide only where
                            // a column is one native 8px text cell. Below that (a pane
                            // whose scale is under 1) a group is WIDER in cells than its
                            // native span, so the blank the game painted immediately
                            // after it — abutting it in native pixels, and therefore
                            // covering nothing of it — maps to a column INSIDE it.
                            //
                            // MEASURED on Shogun's boot menu, `shogun-r322-s890706.z6`,
                            // pane 76x46 at 8x18 (scale 0.95): `START the game` is placed
                            // as 14 columns from 28, and the game's trailing space at
                            // native x=346 — one native cell past the group's last, which
                            // ends at 346 — mapped to column 41 and painted a space over
                            // the final `e`. Likewise `RESTORE a saved gam` and `QUIT the
                            // gam`, on both presses. At scale 1.225 the group UNDER-runs
                            // instead and the blank lands clear, which is why a corpus
                            // checked at 100x40 saw nothing.
                            //
                            // A blank run carries no glyphs and in NATIVE pixels only
                            // ever covers whitespace the glyph run drew itself, so it may
                            // still paint the cells no glyph run claimed (a selection bar
                            // extending past its label) and must skip the rest.
                            let ink: Vec<(i32, i32)> = merged
                                .iter()
                                .filter(|t| !t.text.trim().is_empty())
                                .map(|t| {
                                    let c = run_col(t);
                                    (c, c + t.text.chars().count() as i32)
                                })
                                .collect();
                            for t in &merged {
                                let col = run_col(t);
                                // SQ-0892: the ROW is the run's own native text row
                                // counted from the story box's first — one terminal row
                                // each — not its native pixel through the ring scale.
                                //
                                // This is SQ-0543's packing, which a chrome text STRIP
                                // has had since it was written, applied to the runs that
                                // land INSIDE the story box. It is the stronger case of
                                // the two: these runs are stamped ON the transcript, and
                                // the transcript advances exactly one terminal row per
                                // line, so a run placed by device pixel drifts away from
                                // the very text it overlays as soon as a terminal row
                                // stops being one native 16px text row.
                                //
                                // MEASURED on Shogun's boot menu at a 100x40 pane: the
                                // game prints its three items on consecutive native rows
                                // 21, 22 and 23, and the device mapping put them on
                                // terminal rows 26, 28 and 29 — a skipped row through the
                                // middle of a three-line menu, and `START the game`
                                // rounded to row 26 against a viewport that ceils to 27,
                                // so the first item was clipped off the screen entirely.
                                // A ceil-vs-round disagreement on a shared boundary, which
                                // is the usual shape of a v6 geometry defect. Counting
                                // rows from the box's own top cannot disagree with the
                                // box: the run at the box's first row IS the viewport's
                                // first row, by construction.
                                let row = viewport.y as i32
                                    + (t.y.max(1) as i32 - 1) / i32::from(state.v6_text.cell().h())
                                    - (vp_native.1 as i32) / i32::from(state.v6_text.cell().h());
                                if row < viewport.y as i32
                                    || row >= viewport.bottom() as i32
                                    || col < viewport.x as i32
                                    || col >= viewport.right() as i32
                                {
                                    continue;
                                }
                                // Explicit game colours on the run replace the
                                // theme base per channel; inherited channels
                                // keep it, reverse toggles (SQ-0488).
                                let style = v6_run_style(base, t.fg, t.bg, t.style, TextInk::of(state));
                                let max_w = viewport.right() as usize - col as usize;
                                if max_w > 0 {
                                    // Untrusted game text (SQ-0639).
                                    let text = crate::render::blank_control_chars(&t.text);
                                    if t.text.trim().is_empty() {
                                        // Cell by cell, so the parts of a bar that reach
                                        // past every label still paint (SQ-0898).
                                        for (n, ch) in text.chars().take(max_w).enumerate() {
                                            let c = col + n as i32;
                                            if ink.iter().any(|&(lo, hi)| c >= lo && c < hi) {
                                                continue;
                                            }
                                            buf.set_stringn(c as u16, row as u16, ch.encode_utf8(&mut [0u8; 4]), 1, style);
                                        }
                                    } else {
                                        buf.set_stringn(col as u16, row as u16, text.as_ref(), max_w, style);
                                    }
                                }
                            }
                        }
                        // SQ-1187: park the frame for the next redraw's gate. Stored
                        // last, after every borrow of its canvases has ended.
                        state.graphics_render.borrow_mut().hybrid = Some(frame);
                        return metrics;
                    }
                    // Hint menu open (no streaming story window, SQ-0477): present
                    // the painted screen as positioned terminal text rather than
                    // falling through to the raster composite (an absolutely-
                    // positioned menu rasterizes to an unreadable stamp). The
                    // chrome ring is dropped for this screen — a coherent full-pane
                    // menu. Only when there ARE painted runs; a pure-graphics
                    // no-story frame still falls through to the raster composite.
                    // …on the MACHINE's own page, not the host theme's (SQ-1004).
                    //
                    // This screen is the game's whole page and it names no colour: all
                    // seventy-eight of Arthur's runs carry `fg = bg = 0`, so every cell
                    // they stamp resolves through THIS style, while the cells around
                    // them keep whatever `render_story_pane_frame`'s opening flood put
                    // down — which is the machine's page whenever §8.3's Amiga number
                    // publishes one. Two different grounds on one row: measured on
                    // `Arthur - The Quest for Excalibur.adf` (release 54 / serial
                    // 890606) at a 100x34 pane, `KING LOT` came out `White` on the
                    // theme's `Black` for its eight columns and `Rgb(66, 66, 66)` — the
                    // Amiga page — for the ninety-two beside it. Every line of the hint
                    // menu was its own island.
                    //
                    // RASTER got this right and is why the split is visible at all: it
                    // resolves the same pair through `v6_host_pair`, whose top layer is
                    // that machine pair (SQ-0740), and composes a canvas that censuses
                    // to exactly two colours — 242,239 px of page and 13,761 of ink,
                    // nothing else. `v6_machine_page` is that function's terminal-cell
                    // counterpart, so the two modes now draw one screen.
                    //
                    // A run that names its own colours still wins: `v6_run_style`
                    // overrides each channel a run carries. And `v6_page_pair` is `None`
                    // whenever colours are declined or the profile publishes no pair, so
                    // every other frame keeps the bare theme exactly as before.
                    let status_style = v6_machine_page(state, state.colors.theme.get("upper_window").style);
                    let runs: Vec<&crate::engine::PxText> = layout
                        .chrome
                        .iter()
                        .filter_map(|it| match &it.node {
                            WinNode::Grid(g) => Some(g.px_texts.iter()),
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    // SQ-0711: this path draws the RUNS and nothing else — every
                    // pixel on the screen is discarded. That is right for a screen
                    // that is only text (Zork Zero's InvisiClues, Shogun's boot
                    // menu: both no-story, both with no painted ground). It is
                    // wrong when the game's picture IS the screen. scopa publishes
                    // no Buffer window at all — its screen is three Grids — and
                    // draws its card table entirely with `erase_window` fills, so
                    // hybrid landed here and rendered SEVEN cells ("abort" and
                    // "OK") out of a 100×34 pane while raster drew the table.
                    // A painted ground means there are pixels that only the raster
                    // composite can show, and it draws the runs over them anyway,
                    // so fall through to it.
                    let painted_ground = state.v6_paint.borrow().is_some();
                    if !painted_ground && runs.iter().any(|t| !t.text.trim().is_empty()) {
                        {
                            // Stamp this path like every other exit (SQ-0637): the
                            // painted menu drops the ring, so the next ring frame is a
                            // RESUME and must re-upload the chrome bands (the SQ-0587
                            // gate reads the last path). Leaving "hybrid-ring" standing
                            // here skipped that re-upload and Zork Zero came back from
                            // its InvisiClues menu with the frame art missing — and
                            // `/dump-windows` reported a ring frame that never ran.
                            let mut map = state.v6_cell_map.borrow_mut();
                            map.clear();
                            state.note_v6_path("painted (hint/menu takeover)");
                            map.push(crate::state::V6CellRect {
                                label: "path:painted (hint/menu takeover)".into(),
                                native: (0, 0, 0, 0),
                                cells: (area.x, area.y, area.width, area.height),
                            });
                        }
                        draw_painted_screen(&runs, 0..u16::MAX, 0, area, buf, status_style, TextInk::of(state), &layout.chrome, native.0, state.v6_text.cell());
                        return None;
                    }
                }

                {
                    // Stamp this path too (SQ-0587): otherwise a raster frame leaves
                    // the previous path's record standing and `/dump-windows` reports
                    // the wrong one.
                    let mut map = state.v6_cell_map.borrow_mut();
                    map.clear();
                    state.note_v6_path("raster");
                    map.push(crate::state::V6CellRect {
                        label: "path:raster (full-frame composite)".into(),
                        native: (0, 0, 0, 0),
                        cells: (area.x, area.y, area.width, area.height),
                    });
                }
                // Raster mode (or Hybrid with no story window): rasterize the story
                // text into the clear interior of the native canvas, then draw the
                // whole thing scaled.
                //
                // Generation gate (SQ-0469): the whole canvas rebuild + resize +
                // encode is skipped when nothing that affects the raster changed.
                // `v6_raster_gen` folds every such input into one cheap key; when
                // it matches the last-ready encode we reuse the uploaded protocol
                // and republish the cached scroll metrics — no rebuild, no hash.
                let gen = v6_raster_gen(items, state, area, picker);
                // The game's own page, when it set one (SQ-0532 wave-5). Resolved out
                // here — not inside the gate — because the pane fill below runs on
                // every frame, not just the frames that rebuild the canvas.
                // Gated on the LIVE honor config, like the hybrid flood above: a
                // mid-game `/set-game-colours off` must drop the page even though
                // the model still carries the pair the game set while honored.
                let game_page = if state.config.honor_game_colours {
                    v6::story_bg_rgba(layout.story, &state.colors)
                } else {
                    None
                };
                if state.graphics_render.borrow().v6_wants_build(gen, area) {
                    // SQ-0936: the raster arm takes the SAME locked magnification the
                    // ring does, from the same `locked_scale` — so a title that lands
                    // here sees `v6_pixel_lock` at all, and so the two paths agree.
                    // That agreement is also the check: raster art at a locked scale
                    // should match the ring's, minus the ring's tiling.
                    let fs = picker.font_size();
                    let pane_dev = (
                        u32::from(area.width) * u32::from(fs.width.max(1)),
                        u32::from(area.height) * u32::from(fs.height.max(1)),
                    );
                    // SQ-0978: including the backend gate. Half-blocks resolves the
                    // composite into cells and never sees a device pixel, so a rung
                    // quantized in them is a number the picker invented — the raster
                    // arm asks the same question the ring does, of the same picker.
                    let lock_applies = crate::render::graphics::v6_pixel_lock_applies(picker);
                    state.v6_scale_lock_inapplicable.set(state.config.v6_pixel_lock && !lock_applies);
                    // SQ-1032: the Extended mode asks for a taller canvas at a whole
                    // magnification; every other mode asks for the game's own screen,
                    // which is what `RasterFrame::native` is and what every line below
                    // then does exactly as before.
                    //
                    // Gated on the SAME `lock_applies` — "does this backend have a
                    // device pixel?" (SQ-0978). The extension's height is derived from
                    // `pane_dev`, and on half-blocks the picker's font size is a
                    // hardcoded 10x20 that the encoder then throws away, so a canvas
                    // height measured in those pixels is a number nobody chose.
                    // Half-blocks keeps today's raster composite.
                    let want = if state.config.v6_render == crate::config::V6RenderMode::Extended
                        && lock_applies
                    {
                        v6::RasterFrame::extended(
                            native,
                            pane_dev,
                            state.v6_text.cell(),
                            crate::render::graphics::v6_upscale_cap(picker),
                        )
                    } else {
                        v6::RasterFrame::native(native)
                    };
                    let (canvas, raster_metrics, built) = build_v6_raster_frame(&layout, want, state);
                    // Cache the fresh metrics for skipped frames, then hand the
                    // built canvas to the off-thread resize+encode worker.
                    state.v6_raster_metrics.set(raster_metrics);
                    // An extended frame carries its own whole magnification — one
                    // device pixel per native pixel, times a whole number — which is
                    // strictly finer than any `v6_pixel_lock` rung, so it satisfies the
                    // lock as well. A frame that DECLINED the extension carries none,
                    // and falls back to exactly what `Raster` pins.
                    let lock = built.lock.or_else(|| {
                        (state.config.v6_pixel_lock && lock_applies)
                            .then(|| v6::FrameGeometry::new(native, state.v6_art_scale, state.v6_text.cell()).locked_scale(pane_dev))
                            .flatten()
                            .map(|sc| sc.s)
                    });
                    // The encode is handed the FRAME, not the bare magnification
                    // (SQ-1032): the canvas height, the scale, and the game's own
                    // screen are one subject, and the click map on the far side needs
                    // the third of them to tell a click on the game from a click in
                    // the rows lanthorn added. `built` already carries the height and
                    // the screen; only the lock is re-resolved here, because the pixel
                    // lock is the caller's question and not the frame's.
                    let encoded = v6::RasterFrame { lock, ..built };
                    state.graphics_render.borrow_mut().spawn_v6_encode(picker, canvas, gen, area, encoded);
                }
                // SQ-0532 wave-5: the game's own page runs to the pane EDGE. The
                // composite is drawn letterboxed inside the pane, so the margins
                // around it are ordinary terminal cells — with a game-set page they
                // must carry it too, or a white-page game (Zork Zero) floats its
                // white frame on the dark theme backdrop. A game that sets no page
                // (Journey, Arthur, Shogun) — and `honor_game_colours = false`,
                // where the window's bg stays `Default` — keeps the theme backdrop.
                if let Some(p) = game_page {
                    fill_pane_page(area, p, buf);
                }
                // Draw the last-ready encode (this frame's, or the previous one
                // until the worker lands — never blanks to avoid flicker).
                state.graphics_render.borrow_mut().redraw_v6(picker, area, buf);
                // Publish the raster viewport geometry so the shared scroll
                // keybindings, the [more] pager, and mouse routing engage exactly as
                // in the hybrid/terminal paths (SQ-0455). The rasterized text is a
                // scaled pixel image with no cell-accurate transcript grid, so
                // `transcript_geom.area` is the whole pane and mouse mapping is
                // approximate; the scroll/pager math is exact via the returned
                // `StoryPaneMetrics`. Without a story window (`raster_metrics` unset)
                // there is nothing to scroll — fall through to `None`.
                if let Some(rm) = state.v6_raster_metrics.get() {
                    state.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
                        area,
                        first_abs_row: rm.first_visible_row as usize,
                        total_rows: rm.total_rows as usize,
                    }));
                    return Some(StoryPaneMetrics {
                        scrollbar: false,
                        max_scroll: rm.max_scroll,
                        viewport_rows: rm.viewport_rows,
                        // The raster `[more]` is stamped over the tail of the last
                        // prose row (below), not given a row of its own, so this
                        // path's viewport does not shrink when it shows.
                        prompt_rows: 0,
                        total_rows: rm.total_rows,
                        links: Vec::new(),
                        transcript_surface: true,
                        win_rects: Vec::new(),
                    });
                }
                return None;
            }
            } // !any_overlay_open
            // Cell path with a primary story window. Reached three ways: no image
            // protocol (remote/text-only terminals), a modal overlay is open, or a
            // painted menu takeover was routed here. (SQ-0461 added a fourth — the
            // user asking for this presentation permanently via
            // `v6_render = "frameless"` — and SQ-0895 removed that mode; the path
            // itself is untouched, it simply has one fewer way in.) The v6 native
            // cell geometry is a
            // 40x25-cell postage stamp on a real terminal and pixel art can't
            // render at all, so render like a classic two-window Z-machine
            // game instead — the status window's text rows across the top of
            // the pane (from the chrome grids' pixel runs, classified into
            // left/center/right anchor groups and laid out as a classic
            // full-width status line — SQ-0467), and the story transcript
            // filling everything below at full size, with working
            // metrics/scrollback. (SQ-0186)
            {
                let layout = crate::render::v6_layout::classify_windows(items, state.v6_text.cell());
                // SQ-0906: chrome that names no background of its own sits on the page
                // the GAME dressed the screen with — the story window's own background.
                //
                // This style is the base for all three of this path's stampings: the
                // erase fields (`draw_erase_fills`, whose `ErasedFill` carries `bg = 0`
                // for "the page default"), the anchored status band, and the painted
                // screen that draws a menu's items. It was the bare theme style, so an
                // inherited background resolved to the THEME's — black — however the
                // game had dressed the screen a cell away.
                //
                // Amiga Zork Zero's DEFINE menu is the frame that showed it (release 366,
                // serial 890323). Reached by keys until a line read, `define`, then one
                // character to clear "[Hit any key to continue.]", it is routed here
                // rather than to the hybrid ring by `has_menu && hybrid && !menu_over_art`.
                // Its story window is black on `Standard(10)`, light grey; all 526 of its
                // single-character runs name a foreground of black and no background at
                // all; and its window carries an `ErasedFill`. So the fill and every run
                // alike took the theme's black and the menu rendered black on black — the
                // user's *"very dark and in general doesn't look correct"*.
                //
                // Gated on `honor_game_colours`, because a page the game chose is a game
                // colour: with colours declined the theme owns this exactly as before.
                let status_style = {
                    let s = state.colors.theme.get("upper_window").style;
                    match state.config.honor_game_colours.then(|| {
                        crate::render::v6_layout::story_bg_rgba(layout.story, &state.colors)
                    }) {
                        Some(Some(p)) => s.bg(ratatui::style::Color::Rgb(p[0], p[1], p[2])),
                        _ => s,
                    }
                };
                // Native screen width in cells (v6 screens vary — Zork0 is
                // 320px/40 cells, others differ) sets the anchor thresholds.
                let (native_w, native_h) = crate::render::v6_layout::native_extent(items, &state.v6_text);
                let ncols = (native_w as u32).div_ceil(8).max(1);
                // v6 mouse input in the cell path (SQ-0532/A-F4): this branch draws
                // no game image, so there is no letterbox to invert — but the pane
                // still IS the game's screen, so record the proportional pane→native
                // map. Without one, clicks on this path were simply dead while the
                // raster/hybrid paths' both worked.
                {
                    let cell_px = state
                        .game_picker
                        .as_ref()
                        .map(|p| {
                            let f = p.font_size();
                            (f.width, f.height)
                        })
                        .unwrap_or((8, 16));
                    state.graphics_render.borrow_mut().record_cell_path_click_map(
                        area,
                        (native_w, native_h),
                        cell_px,
                    );
                }
                // Painted text runs across ALL grid windows: the chrome grids
                // carry the status band AND (on a menu/hint screen) the deep
                // absolutely-positioned menu items (Shogun's boot menu paints
                // its three items at native rows 21–23 through window 2).
                let runs: Vec<&crate::engine::PxText> = layout
                    .chrome
                    .iter()
                    .filter_map(|it| match &it.node {
                        WinNode::Grid(g) => Some(g.px_texts.iter()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                if let Some(story) = layout.story {
                    // The cell pane is composed by RELATION to the story
                    // window, never by an absolute native row (SQ-0549/SQ-0491).
                    // A v6 game puts its chrome wherever its artwork leaves room —
                    // Zork0 and Shogun status at rows 0–1, Arthur's at row 12 under
                    // a 12-row art panel, Journey's command menu at rows 19–24
                    // below a story that starts at row 0 — so the three regions are
                    // defined by where the story box IS:
                    //   above it  → the anchored status band, pinned to the pane TOP
                    //   below it  → the command band, pinned to the pane BOTTOM
                    //   inside it → a painted menu overlay at its own native rows
                    // The story transcript fills whatever is left between them.
                    let story_top = state.v6_text.cell().row_of_origin0(story.y_px);
                    let story_bot =
                        ((story.y_px as u32 + story.h_px as u32).div_ceil(16)).min(u16::MAX as u32) as u16;
                    // The band is MEASURED here and PAINTED below, after the
                    // erase fills (SQ-0712): its rows have to be known to size the
                    // story area, but a window's erase is the ground its own text
                    // is painted on, and the fills go down after the transcript.
                    // Painting the band first put advent's status bar under its own
                    // window's erase — the bar vanished the moment the split stopped
                    // leaving window 0 on top of window 1 and the bar became band
                    // text instead of story-box paint.
                    let top_used = anchored_band_rows(&runs, story_top, area.height);
                    // …and WHERE the transcript starts is the STORY WINDOW'S OWN BOX
                    // (SQ-0697), not "wherever the band happens to end". A game that
                    // parked its story window well down the screen left real empty
                    // screen above it, and that gap is part of the layout it
                    // declared. Shogun's title turns on it: the game prints nine
                    // centred lines across native rows 3–11, then moves window 0 to a
                    // 548x64 box at native row 21 — level with, and left of, the
                    // START/RESTORE/QUIT menu at (235,337) — and prints "You may
                    // choose to:" there. Flush-under-the-band put that prompt on the
                    // line below the banner, nine rows above the menu it belongs
                    // beside, which is exactly what a player reported.
                    //
                    // The gap is measured against the chrome's declared BOX, never
                    // its ink: a chrome window taller than the text in it (Zork Zero's
                    // status panel is 78px of which two rows carry runs) has already
                    // been compressed to its inked rows by the band, and re-counting
                    // its own slack as empty screen would push the transcript down for
                    // art this path has deliberately dropped. Nothing above the story
                    // at all → nothing to sit below, so the story keeps the pane's top
                    // edge.
                    let chrome_bot = layout
                        .chrome
                        .iter()
                        .filter(|pw| pw.y_px.saturating_add(pw.h_px) <= story.y_px)
                        .map(|pw| ((pw.y_px as u32 + pw.h_px as u32).div_ceil(16)).min(u16::MAX as u32) as u16)
                        .max()
                        .unwrap_or(story_top);
                    let story_row = (top_used + story_top.saturating_sub(chrome_bot))
                        .min(area.height.saturating_sub(1));
                    // The same displacement, as a signed row delta, for everything
                    // else placed at a native row INSIDE the story box: a menu's
                    // glyphs and the erased ground they sit on travel with it.
                    let story_shift = story_row as i32 - story_top as i32;
                    // The command band below the story (Journey's menu): its own
                    // inked native rows, packed against the pane's bottom edge so
                    // it stays locked there at any pane height instead of floating
                    // at its native row over the story text.
                    let below: Vec<u16> = runs
                        .iter()
                        .filter(|t| !t.text.trim().is_empty())
                        .map(|t| state.v6_text.cell().row_of(t.y))
                        .filter(|&r| r >= story_bot)
                        .collect();
                    let bottom_span = match (below.iter().min(), below.iter().max()) {
                        (Some(&f), Some(&l)) => Some((f, l - f + 1)),
                        _ => None,
                    };
                    let bottom_used = bottom_span
                        .map(|(_, n)| n)
                        .unwrap_or(0)
                        .min(area.height.saturating_sub(story_row));
                    // A chrome GRAPHICS window entirely BESIDE the story (Journey's
                    // half-screen picture column) is story content, not frame art —
                    // this path drops the surrounding chrome, but dropping this lost
                    // the illustration the raster and hybrid paths both show. Give it
                    // its native-proportional column and inset the story beside it.
                    //
                    // WHICH windows those are, and at which pane columns, is
                    // `v6_layout::cell_path_side_columns` and lives there because
                    // `dialog_bounds` has to ask the same question and used to answer
                    // it for itself, on a different measuring basis (SQ-1092).
                    let col_of = |px: u16| (area.width as u32 * px as u32 / native_w.max(1) as u32) as u16;
                    let story_l = story.x_px;
                    let story_r = story.x_px.saturating_add(story.w_px);
                    let sides = crate::render::v6_layout::cell_path_side_columns(&layout, area, native_w);
                    let mut story_x = area.x;
                    let mut story_right = area.right();
                    for s in &sides {
                        if s.left {
                            story_x = story_x.max(area.x + col_of(story_l));
                        } else {
                            story_right = story_right.min(area.x + col_of(story_r));
                        }
                    }
                    let mid_y = area.y + story_row;
                    let mid_h = area.height.saturating_sub(story_row + bottom_used);
                    for s in &sides {
                        let w = s.w.min(area.right().saturating_sub(s.x));
                        let rect = Rect::new(s.x, mid_y, w, mid_h);
                        render_node(&s.win.node, status, char_mode, introspect, state, rect, buf, game_input, links, win_rects, grid_colors);
                    }
                    let story_area = Rect::new(story_x, mid_y, story_right.saturating_sub(story_x), mid_h);
                    {
                        // Which path drew this frame (SQ-0587): the ring records a full
                        // mapping, so without this a cell-path frame would leave the
                        // last ring frame's numbers in `/dump-windows` and read as if
                        // the ring had run.
                        let mut map = state.v6_cell_map.borrow_mut();
                        map.clear();
                        // Say WHY the ring did not run — "it did not" is half an answer.
                        let why = {
                            let modals = state.open_modal_overlays();
                            if !modals.is_empty() {
                                format!("modal overlay open: {}", modals.join(", "))
                            } else if state.game_picker.is_none() {
                                "no image protocol".to_string()
                            } else if has_menu && hybrid && !menu_over_art {
                                "painted menu takeover routed here".to_string()
                            } else {
                                "no story window, or a full-screen picture takeover".to_string()
                            }
                        };
                        state.note_v6_path(&format!("cell — {why}"));
                        map.push(crate::state::V6CellRect {
                            label: format!("path:cell — {why}"),
                            native: (0, 0, 0, 0),
                            cells: (story_area.x, story_area.y, story_area.width, story_area.height),
                        });
                    }
                    let m = render_node(&story.node, status, char_mode, introspect, state, story_area, buf, game_input, links, win_rects, grid_colors);
                    // SQ-0584: erase fields go down over the transcript first — this is
                    // where a painted MENU screen lands (SQ-0484 routes it here out of
                    // hybrid), so without them the menu's text floats over the story it
                    // is supposed to be covering. The cell path is 1:1 with native rows
                    // (8x16 cells), so a window's rect maps by division.
                    draw_erase_fills(
                        &layout.chrome, area, buf, status_style, TextInk::of(state),
                        &|pw: &PositionedWindow| px_rect_to_cells(
                            pw,
                            &crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 },
                            (8, 16),
                            area,
                            story_shift,
                        ),
                    );
                    draw_secondary_buffers(&layout.chrome, area, buf, state, &|pw: &PositionedWindow| {
                        px_rect_to_cells(pw, &crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 }, (8, 16), area, story_shift)
                    });
                    // Chrome text ABOVE the story, as a classic full-width status
                    // line anchored to the pane top. Drawn here, with the rest of the
                    // run stamping and after the erase fills, so a bar sits ON its
                    // own window's erased ground rather than under it (SQ-0712).
                    draw_anchored_status_band(&runs, ncols, story_top, area, buf, status_style, TextInk::of(state));
                    // Painted-screen overlay (SQ-0478): stamp the paint runs INSIDE
                    // the story box as absolutely-positioned terminal text on TOP of
                    // the transcript. A no-op in normal gameplay (chrome grids carry
                    // only the band runs); on a menu screen it draws the items + the
                    // reverse-video selection caret the anchored band drops.
                    draw_painted_screen(&runs, story_top..story_bot, story_shift, area, buf, status_style, TextInk::of(state), &layout.chrome, native_w, state.v6_text.cell());
                    if let Some((first, n)) = bottom_span {
                        // Pack the command band's native rows against the pane
                        // bottom: native `first` lands on the first band row.
                        let shift = area.height as i32 - n as i32 - first as i32;
                        draw_painted_screen(&runs, story_bot..u16::MAX, shift, area, buf, status_style, TextInk::of(state), &layout.chrome, native_w, state.v6_text.cell());
                    }
                    return m;
                }
                // No streaming story window (a painted menu with win0 in paint
                // mode, or none open): the whole pane IS a painted text screen —
                // stamp every run absolutely rather than falling through to the
                // z-ordered cell composite, which renders the native geometry as
                // an unreadable postage stamp (SQ-0478).
                if runs.iter().any(|t| !t.text.trim().is_empty()) {
                    {
                        // Same stamp rule as the story-window arm above (SQ-0637):
                        // this frame did NOT use the pixel path, so the next ring
                        // frame must be treated as a resume.
                        let mut map = state.v6_cell_map.borrow_mut();
                        map.clear();
                        state.note_v6_path("painted (no story window)");
                        map.push(crate::state::V6CellRect {
                            label: "path:painted (no story window)".into(),
                            native: (0, 0, 0, 0),
                            cells: (area.x, area.y, area.width, area.height),
                        });
                    }
                    draw_painted_screen(&runs, 0..u16::MAX, 0, area, buf, status_style, TextInk::of(state), &layout.chrome, native_w, state.v6_text.cell());
                    return None;
                }
            }
            // v6 z-ordered composite (Phase 1b): draw each item in list order —
            // earlier entries (graphics) are background, later entries (text)
            // paint on top. A `Grid` leaf paints only its non-blank cells so an
            // earlier layer shows through the gaps ("cell-text-wins"); other
            // leaves (`Buffer`/`Graphics`) render through the normal recursion.
            let mut result = None;
            for item in items {
                let sub = layered_item_rect(area, item);
                if sub.width == 0 || sub.height == 0 {
                    continue;
                }
                match &item.node {
                    WinNode::Grid(g) => {
                        draw_grid_transparent(g, sub, buf, state.config.honor_game_colours, grid_colors, links, false);
                    }
                    WinNode::Buffer(_) => {
                        // Transparent composite (cell-text-wins for the buffer, like
                        // draw_grid_transparent for grids): render the transcript into
                        // a scratch buffer, then copy only cells with a visible glyph
                        // onto `buf`, so an earlier graphics layer (a full-screen v6
                        // background window) shows through the empty text areas rather
                        // than being painted over by the buffer's opaque bg fill.
                        let mut scratch = Buffer::empty(sub);
                        let m = render_node(&item.node, status, char_mode, introspect, state, sub, &mut scratch, game_input, links, win_rects, grid_colors);
                        result = result.or(m);
                        for yy in sub.top()..sub.bottom() {
                            for xx in sub.left()..sub.right() {
                                let visible = scratch
                                    .cell((xx, yy))
                                    .map(|c| { let s = c.symbol(); !s.is_empty() && s != " " })
                                    .unwrap_or(false);
                                if visible {
                                    if let Some(src) = scratch.cell((xx, yy)).cloned() {
                                        if let Some(dst) = buf.cell_mut((xx, yy)) {
                                            *dst = src;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    WinNode::Graphics(gw) => {
                        // v6 background/overlay window: composite per-cell with
                        // transparency (no grey letterbox, empty canvas paints
                        // nothing) so overlapping v6 windows and the text beneath
                        // stay visible.
                        crate::render::graphics::render_graphics_as_cells(gw, sub, buf, true);
                    }
                    _ => {
                        let m = render_node(&item.node, status, char_mode, introspect, state, sub, buf, game_input, links, win_rects, grid_colors);
                        result = result.or(m);
                    }
                }
            }
            result
        }
    }
}

/// A [`PositionedWindow`]'s absolute cell rect, offset from `area`'s origin and
/// clamped so it never extends past `area`'s bounds (the layered composite's
/// containing rect).
fn layered_item_rect(area: Rect, item: &PositionedWindow) -> Rect {
    let x = area.x.saturating_add(item.x).min(area.right());
    let y = area.y.saturating_add(item.y).min(area.bottom());
    let w = item.w.min(area.right().saturating_sub(x));
    let h = item.h.min(area.bottom().saturating_sub(y));
    Rect::new(x, y, w, h)
}

/// The region a modal dialog should center within: the whole `frame`, minus any
/// Glulx graphics windows.
///
/// Graphics windows are painted through the terminal's own image protocol
/// (kitty/sixel), which draws on top of whatever cells they cover — so a dialog
/// centered over a graphics window is obscured in the real terminal even though
/// it was written into the buffer afterward. This returns the largest rectangle
/// of `frame` that touches no graphics window, so a dialog still spans the story
/// text and the map together where the geometry allows, avoiding only the
/// graphics. `story_area` is where the window tree is laid out (graphics live
/// inside it); pass an empty rect when the story pane isn't shown.
///
/// With no graphics windows this returns `frame` unchanged (today's behavior).
///
/// A Version 6 composite contributes only its side COLUMNS — the one kind of
/// graphics the cell path a modal forces still places. See
/// [`collect_graphics_rects`]'s `Layered` arm (SQ-1092).
///
/// `state` rather than a `&ColorScheme`, because the walk needs two facts that
/// always come from the same place: the theme (whether a pair reserves a separator
/// gutter, SQ-0821) and the machine's `v6_text`, which is where BOTH the character
/// cell `classify_windows` splits on and the unit screen `native_extent` measures
/// come from. Two bare parameters that always travel together is the refactoring
/// policy's tell; `AppState` is the value they travel in, and is what every other
/// render entry point in this file already takes.
pub fn dialog_bounds(model: &ScreenModel, story_area: Rect, frame: Rect, state: &AppState) -> Rect {
    let mut graphics: Vec<Rect> = Vec::new();
    collect_graphics_rects(&model.root, story_area, &mut graphics, state);
    let mut bounds = frame;
    for g in graphics {
        bounds = subtract_rect(bounds, g);
    }
    bounds
}

/// Walk the tree assigning each leaf its terminal rect (exactly as `render_node`
/// does), collecting every graphics leaf's rect.
///
/// `state` is needed for the same reason `render_node` needs it, and for one more:
/// whether a pair reserves a separator gutter is a THEME decision since SQ-0821, and
/// a walk that guessed differently would hand `dialog_bounds` rects one row off what
/// was drawn; and a v6 composite's columns are measured against the machine's own
/// text face (SQ-1092) — see the `Layered` arm.
fn collect_graphics_rects(node: &WinNode, area: Rect, out: &mut Vec<Rect>, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match node {
        WinNode::Pair { vertical, split, border, first, second, .. } => {
            // Reserve the same separator gutter render_node does, so the graphics
            // rects (and thus `dialog_bounds`) match exactly what's drawn — which
            // since SQ-0821 means asking the THEME, not the game's border flag.
            let sep = border.then(|| separator_style(*vertical, &state.colors)).flatten();
            let (a1, _sep, a2) =
                split_area_bordered(area, *vertical, split.fixed, u16::from(sep.is_some()));
            collect_graphics_rects(first, a1, out, state);
            collect_graphics_rects(second, a2, out, state);
        }
        WinNode::Graphics(_) => out.push(area),
        WinNode::Grid(_) | WinNode::Buffer(_) | WinNode::Blank => {}
        // A VERSION 6 COMPOSITE EXCLUDES ONLY WHAT THE CELL PATH STILL PLACES (SQ-1092).
        //
        // `Layered` is built in exactly one place — `session.rs`'s v6 screen model —
        // so this arm is the graphical Z-machine and nothing else. And a v6 frame
        // drawn WHILE A DIALOG IS UP is not the pixel frame: `render_node`'s `Layered`
        // arm takes the ring or the raster composite only
        // `if !state.any_modal_overlay_open()`, and every consumer of what
        // `dialog_bounds` returns is one of the modals that gate names
        // (`overlays::draw_all` centres all of them in this one rect). The frame to
        // reason about here is therefore always the CELL path's, and that path draws
        // no game image — with one exception, the side columns below.
        //
        // Taking every graphics leaf instead did the opposite of both halves. Zork
        // Zero's border window SPANS the story, so the cell path draws nothing for it —
        // and its native cell rect (0, 0, 80, 25) subtracted from an 82x34 frame with
        // the story pane inset one cell left `(0, 26, 82, 8)`, the strip BELOW the
        // stamp. Every dialog centred in that: low, right, and — since
        // `dialog::centered_rect` clamps to its bounds — a fifteen-row leader panel cut
        // to eight with its `Done` button gone.
        //
        // WHICH windows survive, and at which pane columns, is not restated here:
        // `v6_layout::cell_path_side_columns` is the single statement of that rule and
        // the cell path itself is its other caller. Restating it was the defect one
        // layer down — this walk measured the half-pane guard in the GAME's native
        // cells (`PositionedWindow::w`) while the renderer measured it in
        // PANE-PROPORTIONAL ones, which agree only while a pane is about as wide as
        // the ~80-cell v6 screen.
        //
        // The one thing that legitimately differs is the VERTICAL extent, and it
        // differs in the safe direction. The renderer knows the band its column sits
        // in (`mid_y`/`mid_h`, from the run analysis this walk has no part of); the
        // exclusion takes the full pane height, a SUPERSET of what is drawn. Anything
        // narrower would need this walk to redo that analysis, which is the mistake
        // above wearing a different hat — and the guillotine in `subtract_rect` takes
        // the complement of a full-height edge column either way, so a superset here
        // costs the dialog nothing.
        WinNode::Layered(items) => {
            let tf = &state.v6_text;
            let layout = crate::render::v6_layout::classify_windows(items, tf.cell());
            let (native_w, _) = crate::render::v6_layout::native_extent(items, tf);
            for col in crate::render::v6_layout::cell_path_side_columns(&layout, area, native_w) {
                let w = col.w.min(area.right().saturating_sub(col.x));
                out.push(Rect::new(col.x, area.y, w, area.height));
            }
        }
    }
}

/// Collect the window ids of all live graphics windows in the tree.
///
/// Walks `Layered` too, exactly as [`collect_graphics_rects`] does (SQ-0637). A v6
/// composite IS a `Layered` root, and its graphics leaves reach the protocol path
/// whenever the cell path renders one (a chrome column beside the story, or any
/// frame drawn while a modal overlay is open). Omitting them told
/// [`GraphicsRender::retain_live`] that no window was live, so every such frame
/// cleared the whole cache: a full re-encode each frame, and under kitty a full
/// re-transmit under a NEW id whose predecessors were never deleted.
fn collect_graphics_ids(node: &WinNode, out: &mut std::collections::HashSet<u32>) {
    match node {
        WinNode::Graphics(gw) => {
            out.insert(gw.win);
        }
        WinNode::Pair { first, second, .. } => {
            collect_graphics_ids(first, out);
            collect_graphics_ids(second, out);
        }
        WinNode::Layered(items) => {
            for item in items {
                collect_graphics_ids(&item.node, out);
            }
        }
        WinNode::Grid(_) | WinNode::Buffer(_) | WinNode::Blank => {}
    }
}

/// Remove `g` from `bounds` by a guillotine cut, keeping the largest remaining
/// rectangle. If `g` doesn't overlap `bounds`, `bounds` is returned unchanged.
fn subtract_rect(bounds: Rect, g: Rect) -> Rect {
    let ix = g.x.max(bounds.x);
    let iy = g.y.max(bounds.y);
    let ir = g.right().min(bounds.right());
    let ib = g.bottom().min(bounds.bottom());
    if ix >= ir || iy >= ib {
        return bounds; // no overlap
    }
    // The four rectangles of `bounds` lying outside the overlap band.
    let left = Rect::new(bounds.x, bounds.y, ix.saturating_sub(bounds.x), bounds.height);
    let right = Rect::new(ir, bounds.y, bounds.right().saturating_sub(ir), bounds.height);
    let above = Rect::new(bounds.x, bounds.y, bounds.width, iy.saturating_sub(bounds.y));
    let below = Rect::new(bounds.x, ib, bounds.width, bounds.bottom().saturating_sub(ib));
    [left, right, above, below]
        .into_iter()
        .max_by_key(|r| r.width as u32 * r.height as u32)
        .unwrap_or(bounds)
}

/// Whether the leaf touching a pair's separator gutter is a PAINTED graphics
/// window (the game's own drawn divider). `vertical` is the PARENT pair's split
/// orientation; `high` is true when the gutter lies on this node's high-coordinate
/// edge (i.e. this is the pair's `first` child, whose far edge abuts the gutter).
///
/// Walks structurally: along the same split axis only the child on the gutter side
/// touches it; across axes both children span the parent's edge, so either can. Used
/// to suppress our redundant separator when a game (Kerkerkruip) draws its own
/// graphics-window rule there (SQ-0332) — but only when that window is actually
/// painted, so a game's empty frame windows (narco) still get our rule (SQ-0340).
fn edge_touches_painted_graphics(node: &WinNode, vertical: bool, high: bool) -> bool {
    match node {
        // Only a PAINTED graphics window counts as the game's own divider. A
        // window the game opened but never drew into (narco frames its story with
        // empty graphics windows) is NOT a divider — suppressing our separator
        // there would leave the pane with no visible boundary at all. (SQ-0340)
        WinNode::Graphics(g) => g.canvas.pixels().any(|p| p[3] >= 128),
        // A v6 layered composite (Phase 1b) only ever appears as a whole-tree
        // root (built directly by the v6 adapter, never nested inside a Pair
        // sibling), so it can't be the game's own divider here — treat it like
        // the other non-Pair, non-Graphics leaves.
        WinNode::Buffer(_) | WinNode::Grid(_) | WinNode::Blank | WinNode::Layered(_) => false,
        WinNode::Pair { vertical: v, first, second, .. } => {
            if *v == vertical {
                let child = if high { second } else { first };
                edge_touches_painted_graphics(child, vertical, high)
            } else {
                edge_touches_painted_graphics(first, vertical, high)
                    || edge_touches_painted_graphics(second, vertical, high)
            }
        }
    }
}

/// Split `area` for a pair, reserving `border` cells (0 or 1) between the children
/// for the separator rule. `first` gets `fixed` cells; the separator gets `border`;
/// `second` gets the rest. gvm already reserved this 1-cell gutter between bordered
/// siblings, so the two child areas never include it — the rule is drawn in `sep`.
fn split_area_bordered(area: Rect, vertical: bool, fixed: u16, border: u16) -> (Rect, Rect, Rect) {
    if vertical {
        let f = fixed.min(area.height);
        let b = border.min(area.height - f);
        let first = Rect::new(area.x, area.y, area.width, f);
        let sep = Rect::new(area.x, area.y + f, area.width, b);
        let second = Rect::new(area.x, area.y + f + b, area.width, area.height - f - b);
        (first, sep, second)
    } else {
        let f = fixed.min(area.width);
        let b = border.min(area.width - f);
        let first = Rect::new(area.x, area.y, f, area.height);
        let sep = Rect::new(area.x + f, area.y, b, area.height);
        let second = Rect::new(area.x + f + b, area.y, area.width - f - b, area.height);
        (first, sep, second)
    }
}

/// Whether the theme draws an inter-window rule for a split of this orientation,
/// and in which [`BorderStyle`] — `None` for no rule, which also means NO GUTTER
/// IS RESERVED (SQ-0821).
///
/// **Presence is the theme's call, not the game's.** Glk's `winmethod_Border` is
/// the DEFAULT value of that flag, not a considered request, so honouring it drew
/// a rule under the status bar of essentially every Glulx game whether or not its
/// author ever thought about borders. What a game can still do is VETO one:
/// `winmethod_NoBorder` is an explicit statement and is checked by the caller.
///
/// The style comes from the same `upper_window_border` selector the Z-machine
/// status frame uses — `.top` for a horizontal rule (a stacked pair), `.left` for a
/// vertical one (side-by-side) — whose default is [`BorderStyle::None`]. So the
/// shipped default is no rule at all, and `style = "single"` in `style.toml` is how
/// you ask for one. (A dedicated `window-border` selector can follow when the
/// deferred style redesign lands — do NOT add a new selector here.)
fn separator_style(
    vertical: bool,
    colors: &ColorScheme,
) -> Option<crate::render::paneframe::BorderStyle> {
    use crate::render::paneframe::BorderStyle;
    let sides = &colors.upper_window_border_sides;
    let side = if vertical { sides.top } else { sides.left };
    (side != BorderStyle::None).then_some(side)
}

/// Fill every cell of the separator gutter `area` with the Glk inter-window
/// rule: a horizontal run for a stacked/vertical pair (the rule runs across the top
/// child's bottom edge), a vertical run for a side-by-side/horizontal pair.
///
/// Glk provides no border styling, so this reuses the existing themeable
/// window-border presentation rather than a dedicated selector. All three channels
/// come from `upper_window_border`: `style` picks the run glyph (SQ-0821 — it used
/// to draw a hard-coded `─`/`│`, so `style = "double"` parsed, landed on
/// `upper_window_border_sides`, and was never read by this path), the per-side glyph
/// override `.top`/`.left` still beats the style's own glyph, and the colour is the
/// selector's `style`.
///
/// `key_fg`/`key_bg` are the split's KEY (new) window colours (SQ-0325 follow-up),
/// which override the themed colour per channel — but only when the player has
/// asked for the game's colours at all. With `honor_game_colours` off they arrive
/// as `None`, so the theme is authoritative; before SQ-0821 they won regardless,
/// which is one of the three reasons a styled border appeared not to work.
fn draw_window_separator(
    area: Rect,
    vertical: bool,
    border: crate::render::paneframe::BorderStyle,
    key_fg: Option<u32>,
    key_bg: Option<u32>,
    colors: &ColorScheme,
    buf: &mut Buffer,
) {
    let g = &colors.upper_window_border_glyphs;
    let styled = crate::render::paneframe::rule_glyph(border, vertical);
    let glyph = if vertical {
        g.top.as_deref().unwrap_or(styled)
    } else {
        g.left.as_deref().unwrap_or(styled)
    };
    // The separator adopts the split's KEY (new) window colour (SQ-0325 follow-up):
    // draw the rule glyph in `key_fg` on `key_bg` when the game set them, falling
    // back to the themed `upper_window_border` fg/bg per channel when `None`.
    let mut style = colors.theme.get("upper_window_border").style;
    if let Some(rgb) = key_fg {
        style = style.fg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), colors));
    }
    if let Some(rgb) = key_bg {
        style = style.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), colors));
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }
    }
}

/// Draw an inline (non-primary) buffer window's wrapped, styled lines.
fn render_inline_buffer(b: &BufferWindow, state: &AppState, area: Rect, buf: &mut Buffer) {
    // This window's own Normal-style background (Glulx window colour, SQ-0328)
    // replaces the theme transcript bg when the game set one; `None` keeps the
    // theme background (today's behaviour).
    let base = match (b.panel, b.bg) {
        // A game-set window colour always wins.
        (_, Some(rgb)) => state.colors.theme.get("transcript").style.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), &state.colors)),
        // A chrome panel (Scott room panel) uses the themed `room_panel` colour so
        // the split's top and bottom read as distinct regions.
        (true, None) => state.colors.theme.get("room_panel").style,
        (false, None) => state.colors.theme.get("transcript").style,
    };
    fill_style(area, buf, base);
    if b.lines.is_empty() {
        return;
    }
    let kinds = vec![TranscriptKind::Story; b.lines.len()];
    let styles = vec![base; b.lines.len()];
    // Inline images render as bands only when a game picker exists (same as the
    // transcript); `char_px` is that picker's cell pixel size for pixel-accurate
    // fit. Mirrors `render_middle`.
    let images_enabled = state.game_picker.is_some();
    let char_px = state
        .game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width, f.height)
        })
        .unwrap_or((1, 1));
    let (rows, _total, _first) = visible_wrapped_lines_kinded(
        &b.lines,
        &kinds,
        &styles,
        &b.runs,
        &b.para,
        &b.images,
        char_px,
        images_enabled,
        area.height as usize,
        b.scroll,
        area.width,
        None,
    );
    for (i, wr) in rows.iter().enumerate() {
        let row_y = area.y + i as u16;
        // Inline-image band row: blit the strip for this row instead of text
        // (same branch as the transcript draw loop, Task 8).
        if crate::render::inline_image::try_blit_band_row(state, wr, area.x, area.width, row_y, buf) {
            continue;
        }
        draw_str_runs(buf, area.x, row_y, &wr.text, wr.style, &wr.runs, None, area, TextInk::of(state));
    }
}

/// Fill `area` with the transcript background style.
fn fill(area: Rect, buf: &mut Buffer, colors: &crate::colors::ColorScheme) {
    fill_style(area, buf, colors.theme.get("transcript").style);
}

/// Fill `area` with an explicit `style` (used for a per-window background override).
fn fill_style(area: Rect, buf: &mut Buffer, style: ratatui::style::Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// Resolve a themed style's colour to an opaque RGBA for the pixel canvas.
fn style_fg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.fg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Resolve a themed style's background colour to an opaque RGBA for the pixel
/// canvas (the `default_bg` fallback for chrome reverse-video and the story
/// background fill — see [`crate::render::v6_layout::build_chrome_canvas`]).
fn style_bg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.bg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Hardcoded v6 raster default ink (light grey) and page (black), used only when
/// neither the theme nor the terminal (OSC 10/11 probe) supplies a concrete RGB.
const RASTER_FALLBACK_INK: image::Rgba<u8> = image::Rgba([220, 220, 220, 255]);
const RASTER_FALLBACK_PAGE: image::Rgba<u8> = image::Rgba([0, 0, 0, 255]);

/// A style channel's concrete RGB, or `None` when it is unset / a terminal-default
/// or named (non-`Rgb`) colour. The pixel canvas needs real bytes, so only an
/// explicit `Color::Rgb` counts as "the theme supplied this channel".
fn style_rgb(c: Option<ratatui::style::Color>) -> Option<image::Rgba<u8>> {
    match c {
        Some(ratatui::style::Color::Rgb(r, g, b)) => Some(image::Rgba([r, g, b, 255])),
        _ => None,
    }
}

/// Resolve the v6 raster canvas's default ink+page as a matched PAIR from a
/// SINGLE source (SQ-0510, reopened). Resolving the two channels independently
/// paired a theme's ink — a cream/beige foreground authored for the theme's own
/// dark page — with a page from a *different* source (the OSC-probed terminal, or
/// the transparent-canvas-over-white compositor backdrop): two colours never
/// designed to sit together, e.g. beige-on-white. So ink and page are drawn from
/// ONE layer, in order:
///   1. Theme — only when the transcript style supplies BOTH a concrete fg and bg RGB.
///   2. OSC terminal colours — only when the probe answered BOTH channels.
///   3. The hardcoded fallback pair (light grey ink on black page).
///
/// A partial layer (theme fg but no bg; only one OSC channel) is skipped whole
/// rather than mixed, so the returned pair is always internally consistent.
fn v6_default_pair(
    themed: ratatui::style::Style,
    osc_fg: Option<image::Rgba<u8>>,
    osc_bg: Option<image::Rgba<u8>>,
) -> (image::Rgba<u8>, image::Rgba<u8>) {
    if let (Some(fg), Some(bg)) = (style_rgb(themed.fg), style_rgb(themed.bg)) {
        return (fg, bg);
    }
    if let (Some(fg), Some(bg)) = (osc_fg, osc_bg) {
        return (fg, bg);
    }
    (RASTER_FALLBACK_INK, RASTER_FALLBACK_PAGE)
}

/// The HOST's resolved default `(ink, page)` pair for the v6 pixel paths — the
/// transcript theme layered over the OSC probe over the fallback, via
/// [`v6_default_pair`]. One function so the hybrid ring, the raster canvas and
/// the tests can never resolve it differently.
///
/// SQ-0740 adds a layer ABOVE all three: the machine's own screen pair, when the
/// interpreter lanthorn is presenting as has one. Under ZMSD §8.3's Amiga number
/// there is a single pair for the whole screen and it is the machine's, not the
/// terminal's — describing the player's terminal there is precisely what made an
/// Amiga and an IBM PC render identically. `v6_page_pair` is `None` for every
/// other profile and whenever colours are declined, so the three layers below are
/// unchanged for everything else.
pub fn v6_host_pair(state: &AppState) -> (image::Rgba<u8>, image::Rgba<u8>) {
    if let Some((fg, bg)) = state.v6_page_pair.get() {
        return (
            crate::render::v6_layout::packed_to_rgba(fg, RASTER_FALLBACK_INK, &state.colors),
            crate::render::v6_layout::packed_to_rgba(bg, RASTER_FALLBACK_PAGE, &state.colors),
        );
    }
    v6_default_pair(
        state.colors.theme.get("transcript").style,
        state.term_default_colors.fg,
        state.term_default_colors.bg,
    )
}

/// The colour a v6 composite resolves its still-transparent pixels onto: the
/// story window's own declared page when the player is honouring game colours,
/// else the host pair's background.
///
/// ONE rule, asked by both composites (SQ-0944). Raster has flattened onto this
/// since SQ-0510; hybrid needs the same answer the moment a backend that cannot
/// carry alpha starts shipping the ring bands, and a second derivation of "the
/// page" is exactly how the two modes come to disagree about the same frame.
pub(crate) fn v6_composite_page(
    story: Option<&crate::engine::PositionedWindow>,
    default_bg: image::Rgba<u8>,
    state: &AppState,
) -> image::Rgba<u8> {
    v6_game_page(story, state).unwrap_or(default_bg)
}

/// The story window's own declared page, or `None` when the game set none or the
/// player has declined game colours.
///
/// Gated on the LIVE honor config: a mid-game `/set-game-colours off` leaves the
/// recorded pair in the model, and a composite must fall back to the host pair
/// rather than keep resolving onto the game's page.
pub(crate) fn v6_game_page(
    story: Option<&crate::engine::PositionedWindow>,
    state: &AppState,
) -> Option<image::Rgba<u8>> {
    state
        .config
        .honor_game_colours
        .then(|| crate::render::v6_layout::story_bg_rgba(story, &state.colors))
        .flatten()
}

/// The MACHINE's own page alone, as an opaque colour, or `None` when this frame
/// has no machine pair (SQ-0848).
///
/// [`v6_host_pair`]'s top layer without the host fallback under it, which is the
/// distinction that matters to a caller who has a ground of its own to fall back
/// to — `render::inline_image::float_page` layers this BETWEEN the story window's
/// explicit page and the theme, and must not reach past a machine that published
/// nothing into the terminal's default.
pub(crate) fn v6_machine_page_rgba(state: &AppState) -> Option<image::Rgba<u8>> {
    state.v6_page_pair.get().map(|(_, bg)| {
        crate::render::v6_layout::packed_to_rgba(bg, RASTER_FALLBACK_PAGE, &state.colors)
    })
}

/// `base`, with the MACHINE's own ink and page laid under it when this frame has
/// one (SQ-0740) — the terminal-cell counterpart of [`v6_host_pair`], and the same
/// pair.
///
/// This decides only what an INHERITED channel resolves to: text that names its
/// own colours still wins, because [`v6_run_style`] and `draw_str_runs` override
/// each channel a run carries. For Journey under the Amiga profile nothing names
/// any, so this is the whole screen — the frame, the menu and the prose, white on
/// medium grey. Outside that one machine `v6_page_pair` is `None` and `base` is
/// returned untouched, so the host theme owns every other frame exactly as before.
pub(crate) fn v6_machine_page(state: &AppState, base: ratatui::style::Style) -> ratatui::style::Style {
    match state.v6_page_pair.get() {
        Some((fg, bg)) => base
            .fg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(fg), &state.colors))
            .bg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(bg), &state.colors)),
        None => base,
    }
}

/// Build the v6 RASTER composite for one frame, in the game's native pixel space:
/// the chrome art, the story page, the wrapped story text in the game's own ink,
/// its inline floats (drop-caps, room icons), the `[more]` prompt — then flattened
/// onto the page so the shipped image is self-contained. Returns the canvas and
/// the story scroll/pager metrics (`None` when there is no story window).
///
/// Public so a test can assert on the EXACT pixels the render composites (a glyph's
/// ink, the page beneath it, a drop-cap's art) instead of re-implementing the
/// pipeline and pinning the re-implementation. (SQ-0532 wave-5)
pub fn build_v6_raster_canvas(
    layout: &crate::render::v6_layout::V6Layout<'_>,
    native: (u16, u16),
    state: &AppState,
) -> (image::RgbaImage, Option<RasterMetrics>) {
    let (canvas, metrics, _) =
        build_v6_raster_frame(layout, crate::render::v6_layout::RasterFrame::native(native), state);
    (canvas, metrics)
}

/// [`build_v6_raster_canvas`] at a stated frame — the general form, and the one the
/// render calls (SQ-1032).
///
/// `want` is the frame the MODE asked for; the third element of the return is the
/// frame actually BUILT, which is `want` downgraded to
/// [`RasterFrame::native`](crate::render::v6_layout::RasterFrame::native) whenever
/// this particular screen has nowhere to put the extra rows. The caller needs that
/// answer because the magnification it pins the encode to travels with the height
/// (see [`crate::render::v6_layout::RasterFrame`]), and a canvas that declined the
/// extension must keep the letterbox it would have had in `Raster`.
pub fn build_v6_raster_frame(
    layout: &crate::render::v6_layout::V6Layout<'_>,
    want: crate::render::v6_layout::RasterFrame,
    state: &AppState,
) -> (image::RgbaImage, Option<RasterMetrics>, crate::render::v6_layout::RasterFrame) {
    use crate::render::v6_layout as v6;
    // The game's own screen. Everything between here and the flank extension is
    // stated in it and is unchanged by SQ-1032: the extension only ever adds rows
    // BELOW, and the game laid its windows out on this.
    let native = want.native;
    let (default_fg, default_bg) = v6_host_pair(state);
    // The story PAIR (SQ-0510, extended in SQ-0532 wave-5): a game-set
    // story-window colour (`set_colour`) wins per channel, else the paired
    // host default. Zork Zero boots `set_colour(fg=2, bg=9)`, so taking its
    // white page while rasterizing the prose in the host's own light default
    // ink drew white-on-white — unreadable. The window's explicit fg wins over
    // `default_fg` exactly as its bg wins over `default_bg`. Both are gated on
    // the LIVE honor config: a mid-game `/set-game-colours off` leaves the
    // recorded pair in the model, and the composite must fall back to the host
    // pair rather than keep painting the game's page/ink.
    let honor = state.config.honor_game_colours;
    let game_ink = if honor { v6::story_fg_rgba(layout.story, &state.colors) } else { None };
    let game_page = v6_game_page(layout.story, state);
    let page = game_page.unwrap_or(default_bg);
    let ink = game_ink.unwrap_or(default_fg);
    // What the story box is measured against (SQ-0728): the same layers, MINUS the
    // chrome text. `story_clear_native` shrinks the story window edge by edge until
    // no edge touches an opaque pixel, and its purpose is to seat the prose inside
    // bordering frame ART — Zork Zero's ring, Arthur's plate, Journey's picture
    // panel. Rasterized glyphs are opaque too, so a chrome window the game paints
    // INSIDE window 0 was eating the story box instead of coexisting with it, which
    // is not what a real interpreter does: Shogun's title prints "You may choose
    // to:" at x=47 while its menu window prints "START the game" at x=235, on the
    // same rows. Measured against the full canvas Shogun's declared 548x64 box came
    // back as 548x16 — one row, which `build_main_text` then reports as ZERO visible
    // rows — and Journey's 392x304 text panel came back 392x0. Against the art it is
    // the box each game declared. Same lesson as `build_graphics_canvas` on the
    // hybrid side (SQ-0500): "opaque" is not "artwork".
    //
    // And it is the ART and NOTHING ELSE (SQ-1056). A GROUND is not artwork either,
    // and the probe walks the story window's own edges inward, so the only pixels it
    // can ever read are the ones INSIDE that window — which makes any ground laid
    // there a self-obstruction. `fill_window_pages` has always known this and skips
    // every window overlapping the story box for exactly this reason; the painted
    // ground (`blit_paint_ground`, the game's `erase_window` fills) had no such rule,
    // and a game that erases its OWN story window shrinks the box to nothing:
    //
    //   stories/Shogun.toast (Macintosh r292/890314), leaving InvisiClues with `q`
    //     against art             (46, 30) 548x370   — the box the game declared
    //     + the painted ground   (230, 215) 180x0    — degenerate, so SQ-0578's
    //                                                  `w >= 8 && h >= 16` floor
    //                                                  ships a chrome-only canvas
    //
    // The ground grew by 202,760 px on that one frame — 548x370, the story window to
    // the pixel — so the screen came back with its score bar and both ornaments and
    // no prose at all, and only `restart` recovered. Hybrid was unaffected because it
    // measures against `build_graphics_canvas` alone (`frame_art`, SQ-0896); this is
    // now the same oracle on both paths, which is the only reason they can agree.
    // SQ-0894 said as much when it measured the corpus "against the ART-ONLY canvas
    // (the oracle §3(b) says it needs)".
    let obstruction = v6::build_graphics_canvas(&layout.chrome, native);
    // SQ-0793: …and the side border art is extended to the bottom of that native
    // screen before anything is scaled, exactly as the hybrid ring extends it
    // (SQ-0698). The chrome runs come along because a game with a command menu
    // under its story window has no border to extend (SQ-0819). See
    // `extend_raster_flanks`.
    let chrome_runs: Vec<&crate::engine::PxText> = paint_runs(&layout.chrome).collect();
    // SQ-0578: only stamp the story when its clear interior can hold at least
    // one full 8x16 text cell. A full-screen picture (Zork Zero's rebus) grows
    // window 0 over the whole screen and paints art across virtually all of it;
    // the inset then leaves a degenerate sliver (the rebus leaves 0x80), and the
    // `.max(1)` below pinned that to a ONE-COLUMN story box — the whole
    // transcript re-wrapped a character per line with a [more] prompt that took
    // hundreds of keypresses to drain. No cell fits → the picture owns the
    // screen: ship the art alone and report no scroll metrics, exactly like the
    // no-story-window case.
    //
    // SQ-1032 HOISTED this above the flank extension, where it used to sit just
    // below, because the frame's HEIGHT is decided from it. It reads `obstruction`
    // and nothing else, and nothing between the two positions touches that canvas,
    // so `Raster` builds the identical composite either way.
    let cell = state.v6_text.cell();
    let story_clear =
        v6::story_clear_native(layout.story, &obstruction).filter(|&(_, _, w, h)| w >= 8 && h >= 16);
    // SQ-1032: does this frame extend, and by how many native rows?
    //
    // The MODE asked for it (`want`); this screen has to be able to SPEND it. Every
    // test below is a screen where the extra rows would have no prose to hold, and a
    // canvas grown past the artwork with nothing in it is a strip of bare page under
    // the game's own frame. They are the same questions, in the same order, that the
    // composite asks itself further down — asked here so the frame cannot grow down a
    // path that then declines to draw into it.
    //
    // The answer is ONE value (SQ-1132): `None` declines, and `Some(windows)` extends
    // by `want.extension()` and names the chrome windows that travel down with the
    // frame's bottom edge — usually none. The two are one decision, because a band
    // that cannot be moved is a frame that cannot grow.
    let anchored = 'ext: {
        if want.extension() == 0 {
            break 'ext None;
        }
        let Some(story) = layout.story else { break 'ext None };
        // A text-only command strip under the story window — Journey. Hybrid meets
        // this case by BOTTOM-ANCHORING the strip and letting the story fill between
        // (`BottomPlan::Menu`), and this mode cannot: the composite is one image
        // built in the game's own coordinates, so moving the game's chrome inside it
        // is a composition change rather than a layout one. The flank extension
        // declines the identical frame for the identical reason (SQ-0819), so
        // extending here would strand the menu mid-canvas over an unextended flank.
        // Declining leaves Journey exactly as `Raster` draws it.
        if menu_strip_below_story(story, &obstruction, &layout.chrome, native, cell) {
            break 'ext None;
        }
        // …and ANY chrome the game put below its story window, which the test above
        // does not see. `menu_strip_below_story` answers false as soon as the story
        // reaches within one native text row of the screen bottom — and that is
        // exactly when Arthur spends the row it forgives. Measured on
        // `arthur-r74-s890714.z6` (release 74) and reproduced on
        // `Arthur - The Quest for Excalibur.adf` (release 54 / serial 890606): window
        // 0 is (28, 208) 584x176 so its bottom is 384, window 3 is laid across native
        // (28, 384, 584, 16) — the LAST text row of a 640x400 screen — and prints
        // *"You don't need to use the word …"* or *"If only you had a crystal
        // ball…."* into it. `native.1 (400) <= 384 + 16` holds, so the menu test
        // returns false before it looks.
        //
        // **The frame does not decline; the band travels with it** (SQ-1132). It used
        // to decline, on the reading that the composite cannot bottom-anchor anything
        // — and Arthur then made the whole screen change SIZE every time the parser
        // failed, because that band is TRANSIENT: window 0 is 584x**192** and window 3
        // empty on a turn the game understood, 584x176 with one run in window 3 on a
        // turn it did not. A frame height that depends on whether the last command
        // parsed is not a rendering decision anyone made.
        //
        // Bottom-anchoring one window IS available to the composite, because the
        // arithmetic already leaves room for it: the prose box grows to
        // `story_bottom + extension`, which is `canvas_h` minus exactly the native
        // rows between the story window's bottom and the game's screen bottom — the
        // band's own height. So a band moved down by the extension lands in the gap
        // the extension opened, at the same distance from the frame's bottom edge the
        // game put it at from the screen's. That is what hybrid's `BottomPlan::Menu`
        // does, reached by moving a window rather than by re-composing the image.
        //
        // Journey is untouched: it declines one test earlier, on `menu_strip_below_
        // story`, and the flank extension declines the identical frame (SQ-0819), so
        // a Journey frame would grow a menu over an unextended flank. This arm is for
        // a band the flanks tile straight past.
        let anchored = if menu_band_rows(&menu_band_runs(&chrome_runs, story), cell) > 0 {
            // The band has to move as WHOLE WINDOWS — page, cell grid and runs
            // together — and the canvas it moves onto has to be expressible as a
            // screen height. Anything else is unrecognised, and unrecognised declines
            // exactly as it did before (CLAUDE.md: skip rather than guess).
            match bottom_anchored_chrome(&layout.chrome, story, native) {
                Some(ws) if u16::try_from(want.canvas_h).is_ok() => ws,
                _ => break 'ext None,
            }
        } else {
            Vec::new()
        };
        // A story window enclosed by its own art is a CANVAS, not a page (SQ-0729):
        // fmvpoker gets no transcript at all, so there is no prose to grow.
        if story_window_is_a_canvas(layout, native) {
            break 'ext None;
        }
        // A `Grid` in the story slot contributes its rect and nothing else
        // (SQ-1026) — again no transcript. scopa and Amiga Shogun's InvisiClues.
        if !matches!(&story.node, WinNode::Buffer(_)) {
            break 'ext None;
        }
        // The picture owns the screen (SQ-0578), or an absolutely-placed plate is
        // drawn INSTEAD of prose (SQ-0707). Arthur's intro plate is the second.
        let Some(clear) = story_clear else { break 'ext None };
        if v6::story_prose_box(clear, layout.story_gfx, cell).is_none() {
            break 'ext None;
        }
        Some(anchored)
    };
    let extension = if anchored.is_some() { want.extension() } else { 0 };
    let frame = if extension == 0 { v6::RasterFrame::native(native) } else { want };
    // SQ-1132: the chrome the COMPOSITE draws. It is the game's own chrome on every
    // frame but one — the frame that extends past a band the game anchored below its
    // story window, where that band's windows are re-seated `extension` native rows
    // lower so they keep their distance from the frame's BOTTOM EDGE. Every consumer
    // below reads this list rather than `layout.chrome`, so the band's page, its cell
    // grid, its runs and the rects the prose spares cannot disagree about where it is.
    //
    // `layout.chrome` still answers every question stated in the GAME's screen — the
    // art canvas, the menu tests above, the flank extension — because those are about
    // the screen the game laid out, which the extension never changes.
    //
    // One consequence, stated rather than discovered: a moved band is drawn in rows
    // the game's screen does not have, so `V6ClickMap` — which bounds a click by
    // `screen` and drops anything below it — will not report a click on it. Arthur's
    // parser error is output and nothing else; a CLICKABLE band under a story window
    // is Journey's, and Journey declines the extension one test earlier.
    let moved: Vec<crate::engine::PositionedWindow> = match &anchored {
        Some(ws) if extension > 0 => {
            ws.iter().map(|&i| bottom_anchor(layout.chrome[i], extension, cell)).collect()
        }
        _ => Vec::new(),
    };
    let mut chrome: Vec<&crate::engine::PositionedWindow> = layout.chrome.clone();
    if let Some(ws) = &anchored {
        for (k, &i) in ws.iter().enumerate().take(moved.len()) {
            chrome[i] = &moved[k];
        }
    }
    // A window drawn below the game's own screen needs a canvas that reaches it, so a
    // frame carrying one is built at the FRAME's height instead of being grown into it
    // afterwards. `native` is the chrome canvas's SIZE and nothing else, and the fit
    // in `u16` was settled with the decision above.
    let canvas_native =
        if moved.is_empty() { native } else { (native.0, frame.canvas_h as u16) };
    // Raster has no cells to draw text with, so it needs every run imaged: the
    // empty set is not a default here, it is this path's answer (SQ-0903).
    let mut canvas =
        v6::build_chrome_canvas(&chrome, canvas_native, default_fg, default_bg, &state.colors, v6::TextLayer::All, &state.v6_text);
    // …and the lines of any SECONDARY prose window (SQ-0729), which the chrome
    // canvas does not draw. The story page below spares them like any chrome text.
    // …and the live input line into whichever of them the player is typing into
    // (SQ-0746), on the same "only when the view is at the bottom" rule
    // `build_main_text` applies to the transcript's own live line.
    let panel_input =
        (state.effective_transcript_scroll() == 0).then_some(state.input.value.as_str());
    v6::draw_secondary_prose(&mut canvas, &chrome, ink, honor, &state.colors, panel_input, &state.v6_text);
    // SQ-0704: each chrome window's own page (ZMSD §8.8.3.2) fills its unpainted
    // pixels before the story is stamped — the story box itself is skipped (see
    // `fill_window_pages`). This runs on the COMPOSITE only: the clear-interior
    // probe below reads the art canvas, which no ground ever touches (SQ-1056).
    // The game's own painted ground — erase_window fills (SQ-0706) — goes UNDER
    // the art and glyphs already on the canvas and BEFORE the window pages claim
    // what is left, because a fill is the oldest thing on the screen: the game
    // filled its rectangle, then printed the label on top of it.
    let grounds = |c: &mut image::RgbaImage| {
        v6::blit_paint_ground(c, state.v6_paint.borrow().as_deref(), v6::TextLayer::All, state.v6_text.cell());
        if honor {
            v6::fill_window_pages(
                c,
                &chrome,
                layout.story,
                &state.colors,
                v6::TextLayer::All,
                state.v6_text.cell(),
            );
        } else {
            // SQ-0716: colours declined, but a window the game has PAINTED INTO still
            // gets its page — scopa's felt table is a full-screen `erase_window` in
            // explicit green that `drain_erase_fills` drops as a screen clear, so
            // window 1's background is the only surviving record of that drawing.
            // Gating it left a black table under the game's own green stripes and
            // cards. See `fill_painted_window_pages`.
            v6::fill_painted_window_pages(
                c,
                &chrome,
                layout.story,
                &state.colors,
                state.v6_paint.borrow().as_deref(),
                state.v6_text.cell(),
            );
        }
    };
    grounds(&mut canvas);
    // …unless it was already built at the frame's height, because a bottom-anchored
    // band had to be drawn into rows the game's own screen does not have.
    if extension > 0 && moved.is_empty() {
        canvas = grow_canvas_rows(canvas, frame.canvas_h);
    }
    extend_raster_flanks(&mut canvas, &obstruction, layout.story, &layout.chrome, frame, cell);
    let mut raster_metrics: Option<RasterMetrics> = None;
    if let Some((sx, sy, sw, sh)) = story_clear {
        // Paint the story page opaque (SQ-0510, reopened). Leaving it
        // transparent let whoever composites the image pick the colour
        // instead of us. `story_clear_native` has already inset past any
        // bordering frame art, and `flatten_onto_page` below covers the
        // degenerate case where that inset leaves nothing; inline-image
        // floats redraw on top in `draw_story_text`. So no artwork is covered.
        // The chrome TEXT the game printed inside the box is spared (SQ-0728):
        // window 0's page is under it, not over it — Shogun's title prints its
        // menu into window 0's box and both belong on the screen.
        v6::fill_story_page_under_chrome_text(
            &mut canvas,
            (sx, sy, sw, sh),
            page,
            &chrome,
            state.v6_paint.borrow().as_deref(),
            &state.v6_text,
        );
        // …then the story window's OWN absolutely-placed artwork, before any
        // prose: Arthur's intro centres a 584×392 plate in window 0, so the plate
        // is the page's backdrop, not part of the frame ring — and the page fill
        // just above would otherwise wipe it. The probe for the clear interior ran
        // BEFORE this blit, so the text box is still measured against the frame
        // art alone. (SQ-0695)
        v6::blit_story_gfx(&mut canvas, layout.story_gfx);
        // A story window whose own art ENCLOSES it is a CANVAS, not a page
        // (SQ-0729): what it shows is the runs sitting on it, at the coordinates
        // the game's own `set_cursor` named, and a scrolling re-render of
        // everything it ever printed is the wrong reading of the window. So its
        // live runs are painted and there is no transcript on this frame — no
        // prose box, and no scroll metrics, exactly as when a plate owns the
        // screen. See `story_window_is_a_canvas`: fmvpoker alone.
        if story_window_is_a_canvas(layout, native) {
            v6::draw_story_canvas_runs(&mut canvas, layout.story, ink, page, honor, &state.colors, &state.v6_text);
            return finish_v6_raster_canvas(canvas, page, raster_metrics, frame);
        }
        // **A `Grid` in the story slot contributes its RECT and nothing else**
        // (SQ-1026). With no primary `Buffer` on the frame, `classify_windows`
        // falls back to the `Grid` filling the clear middle of a ring of artwork —
        // it wants the rect, so the ring has a viewport to lay out around, and the
        // grid stays in `chrome` so its own runs still reach the canvas. Every
        // reader that wants a BUFFER pattern-matches for one and declines
        // otherwise; this was the one that did not, and the host transcript is the
        // most buffer-shaped thing there is.
        //
        // Amiga Shogun r295/890321 is the report. Its InvisiClues screen publishes
        // no buffer at all — a full-screen graphics frame plus three grids — so
        // window 0 resolved to the 500x330 topic list at native (70, 70) and the
        // whole scrollback was re-wrapped into it: 78 rows of it, under the topics.
        // The tell that settled it was the player's own `/dump-windows` output
        // appearing inside the menu, which only the HOST transcript can put there.
        //
        // No prose box and no scroll metrics, exactly as when a plate owns the
        // screen — there is no transcript on this frame.
        if !matches!(layout.story.map(|s| &s.node), Some(WinNode::Buffer(_))) {
            return finish_v6_raster_canvas(canvas, page, raster_metrics, frame);
        }
        // Whether any prose belongs on THIS frame, and where (SQ-0707). An
        // absolutely-placed plate is drawn INSTEAD of prose, not under it: the
        // game erases, draws, and waits, so the narration is its own picture-less
        // screen. `None` = the plate owns the screen, and rasterizing scrollback
        // onto it would paint the PREVIOUS screen's text across the art.
        let Some((tx, ty, tw, th)) = v6::story_prose_box((sx, sy, sw, sh), layout.story_gfx, cell) else {
            return finish_v6_raster_canvas(canvas, page, raster_metrics, frame);
        };
        // Window-0 inline pictures (drop-caps, room icons) arrive as
        // transcript-anchored floats (`transcript_images` sidecar):
        // build_main_text wraps text beside them and draw_story_text
        // blits each at its anchored row — they scroll with the text.
        // Non-square 8×16 v6 cell (SQ-0479): columns divide the
        // clear width by the cell's width, rows the height by its height.
        let (sx, sy) = (tx, ty);
        // SQ-1032: the extension belongs to the PROSE BOX and to nothing else. It is
        // whole text rows of the game's own face by construction
        // (`RasterFrame::extended` sizes it in `cell.h`), so `rows` below gains
        // exactly that many and keeps whatever sub-row remainder the unextended box
        // already had — the region lands on the face's own grid rather than being
        // letterboxed inside itself.
        //
        // `th + extension` rather than "grow to the canvas bottom", which is the
        // literal transcription of hybrid's `area.bottom() - y`: on every title that
        // extends they are the same number (the story box reaches the game's screen
        // bottom), and this one additionally cannot walk prose over a bottom band the
        // game drew below its own story window.
        let th = th + extension;
        let cols = (tw / u32::from(cell.w())).max(1) as u16;
        let rows = (th / u32::from(cell.h())).max(1) as u16;
        let (main, rm) = build_main_text(state, cols, rows);
        // …sparing the cells another window's own text already holds (SQ-0729).
        // The page fill above spares them; the GLYPHS did not, so the transcript
        // was drawn straight through them. fmvpoker's dealt hand is the report:
        // its five cards fill the frame's interior, window 0's clear rectangle
        // drops onto the box the game gave its bottom prose window, and the boot
        // banner landed on top of "You draw (a) an Eight, (b) a Three, …" — the
        // line the player needs in order to see their draw. The transcript is the
        // host's re-render of window 0's whole history; the label is on the screen
        // now, so the label wins.
        // The momentary word reveal, when one is lit (SQ-1138). Resolved here and
        // applied at blit time rather than as a pass over the finished canvas:
        // there are no cells to re-style, and re-walking the pen afterwards to
        // find where each word landed would be a second copy of this function's
        // layout arithmetic — the hand-maintained cross-file invariant the
        // refactoring policy exists to forbid.
        //
        // The fallback ink is the story's own, which makes a theme that cannot
        // resolve to concrete bytes draw the prose exactly as it already was
        // rather than in some colour nobody chose.
        let reveal = crate::reveal::raster_reveal(state, ink);
        v6::draw_story_text(
            &mut canvas,
            &main,
            sx,
            sy,
            cols,
            rows,
            ink,
            &v6::chrome_text_rects(&chrome, &state.v6_text),
            &state.v6_text,
            reveal.as_ref(),
        );
        // [more] pager indicator (SQ-0455): when a single turn's output
        // overflowed the story box the shared pager (SQ-0404) parks the
        // scroll and shows a `[more]` prompt. The raster path can't reserve
        // a terminal row, so draw the prompt as a text run bottom-right of
        // the story box, themed via the `more_prompt` selector (drawn as a
        // reverse-video block, matching the terminal bar).
        if state.pager.active {
            let mp = state.colors.theme.get("more_prompt").style;
            // Reverse-video against whatever the story page/ink actually ARE.
            // When the game set its own pair (Zork Zero's black on white) the
            // prompt must reverse THAT pair, or a themed block resolved from an
            // unrelated source lands as (say) white on white on the game's page.
            // With no game pair the themed `more_prompt` selector still governs,
            // resolved as a PAIR from one source (theme both / OSC both /
            // fallback) so the block and its ink never mix sources.
            let (block, prompt_ink) = match (game_page, game_ink) {
                (Some(p), Some(i)) => (i, p),
                _ => v6_default_pair(mp, state.term_default_colors.fg, state.term_default_colors.bg),
            };
            let label = "[more]";
            let cell = state.v6_text.cell();
            let last_row = rows.saturating_sub(1) as u32;
            // SQ-1009: flush RIGHT by the PEN's width, and stepped by the pen. Sized
            // by a character count against the cell it drew a reverse block the
            // glyphs no longer filled — six narrow letters rattling inside six
            // 8-px slots, which is what "compressed and unreadable" looks like from
            // the other side. Both numbers come from the same pen, so they cannot
            // disagree.
            let width = state.v6_text.run_px(label);
            let mut pen = sx + (cols as u32 * u32::from(cell.w())).saturating_sub(width);
            for ch in label.chars() {
                let adv = state.v6_text.advance(ch);
                crate::render::bitfont::blit_glyph(
                    &mut canvas, ch, pen, sy + last_row * u32::from(cell.h()), adv, u32::from(cell.h()), prompt_ink, Some(block), Some(&state.v6_text),
                );
                pen += adv;
            }
        }
        raster_metrics = Some(rm);
    } else {
        // No usable text box — the picture owns the screen (SQ-0578), or there is
        // no story window at all. The story window's own plate still has to ship.
        v6::blit_story_gfx(&mut canvas, layout.story_gfx);
    }
    // Every layer has now drawn. Raster mode ships the WHOLE canvas as
    // one image, so any pixel still fully transparent would be resolved
    // by the compositor, not by us — kitty keeps the alpha and lets the
    // terminal decide, halfblocks flattens an untouched cell's
    // `Color::Reset` to WHITE. Paint those leftovers (the letterbox
    // margins around the frame art, and the story interior itself if a
    // full-bleed background tint inset `story_clear_native` to nothing)
    // with the same page, so the composite is self-contained and looks
    // identical on every protocol/terminal. Touches alpha==0 pixels
    // ONLY — art, status bands, glyphs and drop-caps are all opaque and
    // are left byte-for-byte alone. (SQ-0510)
    finish_v6_raster_canvas(canvas, page, raster_metrics, frame)
}

/// Seal a v6 raster composite: resolve every still-transparent pixel to the story
/// page so the image is self-contained (SQ-0510). Shared by the normal tail of
/// [`build_v6_raster_canvas`] and its plate-owns-the-screen early return (SQ-0707).
fn finish_v6_raster_canvas(
    mut canvas: image::RgbaImage,
    page: image::Rgba<u8>,
    raster_metrics: Option<RasterMetrics>,
    frame: crate::render::v6_layout::RasterFrame,
) -> (image::RgbaImage, Option<RasterMetrics>, crate::render::v6_layout::RasterFrame) {
    crate::render::v6_layout::flatten_onto_page(&mut canvas, page);
    (canvas, raster_metrics, frame)
}

/// SQ-1032: the same composite with more transparent native rows below it.
///
/// Transparent, not paged: the flank extension paints its own columns into them and
/// `flatten_onto_page` resolves whatever is left to the story page, which is exactly
/// how every other unpainted pixel on this canvas reaches the screen (SQ-0510).
fn grow_canvas_rows(src: image::RgbaImage, height: u32) -> image::RgbaImage {
    if height <= src.height() {
        return src;
    }
    let mut out = image::RgbaImage::new(src.width(), height);
    for y in 0..src.height() {
        for x in 0..src.width() {
            out.put_pixel(x, y, *src.get_pixel(x, y));
        }
    }
    out
}

/// Flood every cell of `area` with an opaque game page colour (SQ-0532 wave-5).
///
/// Used by the two v6 pixel modes when the story window carries an EXPLICIT
/// background: the game's page is the whole screen's page, so the pane must show
/// it everywhere the scaled composite doesn't reach (letterbox margins) and
/// everywhere the composite is transparent (the chrome ring's clear pixels).
/// Drawn first, so the ring, the story viewport and the raster image all paint
/// over it.
fn fill_pane_page(area: Rect, page: image::Rgba<u8>, buf: &mut Buffer) {
    let style = ratatui::style::Style::new().bg(ratatui::style::Color::Rgb(page[0], page[1], page[2]));
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// SQ-1187: the generation key for the whole hybrid chrome ring — the twin of
/// [`v6_raster_gen`], covering every input `build_hybrid_frame` reads. When it
/// is unchanged the entire ring compute (both native canvases, the layout
/// plans, the strip carving, the flank probes) is skipped and the cached
/// [`HybridFrame`] is replayed.
///
/// **A missed input here is a stale-frame bug that looks entirely
/// self-consistent** — this repo's signature defect — so the coverage is
/// deliberately generous: the model's render fields are observed directly
/// (never a hand-maintained mutation counter), and everything the compute
/// resolves through a global (the zvm palette) or through the theme (the
/// default pair, the ANSI palette) is folded in by VALUE.
///
/// Deliberate exclusions, each because the draw half re-reads it live on every
/// frame (cached or not), so the composed output cannot go stale on it:
/// - transcript content/generation, the input line, scroll, focus, the pager,
///   the reveal, `char_mode`: the story viewport is drawn as live terminal
///   cells by `render_node`/`render_transcript` each frame; none of them
///   reaches the ring compute.
/// - `config.v6_render`: the gate lives inside the Hybrid branch — a mode
///   change re-routes above it before this key is ever computed.
/// - modal overlays and the menu-takeover routing: decided upstream of the
///   gate, per frame.
/// - Buffer erase/fill freshness (`draw_erase_fills`) and secondary-buffer
///   text placement: drawn per frame from the live model (non-primary LINES
///   are keyed, because the ring's panel rects depend on their emptiness).
/// - theme selectors beyond the default pair + ANSI palette: `upper_window`
///   (via `base`), `TextInk`, separators — all resolved in the draw half per
///   frame; the canvas itself resolves colours only through the keyed pair,
///   the fixed §8.3.1 tables, the keyed zvm palette and the keyed theme
///   palette.
/// - face BITMAPS beyond `wrap_fingerprint` + the cell: faces are resolved at
///   mount and only replaced wholesale; the metric fingerprint covers the
///   advances (the raster key carries no face fact at all — this carries more).
/// - `GraphicsWindow::upscale`: a Scott/Glulx rendering hint the v6 layered
///   path never reads.
pub fn v6_hybrid_gen(
    items: &[PositionedWindow],
    state: &AppState,
    area: Rect,
    picker: &ratatui_image::picker::Picker,
    story: &PositionedWindow,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Pane geometry + device cell + backend. The backend decides tile shape,
    // glyph-over-art layering, alpha flattening and the pixel-lock ladder.
    (area.x, area.y, area.width, area.height).hash(&mut h);
    let fs = picker.font_size();
    (fs.width, fs.height).hash(&mut h);
    (picker.protocol_type() as u8).hash(&mut h);
    // The v6 window model: every window's box geometry plus its render content
    // — graphics by version stamp (not pixels), text by its positioned runs and
    // colours. This observes the composited output, so any paint/erase/scroll
    // or colour change on the zvm side is captured without a bespoke counter.
    for pw in items {
        (pw.x, pw.y, pw.w, pw.h, pw.x_px, pw.y_px, pw.w_px, pw.h_px, pw.left_margin, pw.right_margin).hash(&mut h);
        match &pw.node {
            WinNode::Graphics(g) => {
                g.win.hash(&mut h);
                g.version.hash(&mut h);
                (g.canvas.width(), g.canvas.height()).hash(&mut h);
            }
            WinNode::Grid(g) => {
                (g.bg, g.fg, g.cursor, g.cursor_active).hash(&mut h);
                for t in &g.px_texts {
                    (t.x, t.y, t.style, t.fg, t.bg, t.grow, t.gcol).hash(&mut h);
                    t.text.hash(&mut h);
                }
            }
            WinNode::Buffer(b) => {
                (b.bg, b.fg, b.primary).hash(&mut h);
                // A SECONDARY prose window's lines decide the ring's panel
                // rects (and are drawn by the ring's neighbours), so a change
                // to them must rebuild.
                if !b.primary {
                    b.lines.hash(&mut h);
                }
            }
            _ => {}
        }
    }
    // The painted ground (SQ-0706), hashed by CONTENT exactly as the raster key
    // does: it is blitted into the chrome canvas and described by no window.
    match state.v6_paint.borrow().as_deref() {
        Some(ground) => {
            1u8.hash(&mut h);
            ground.dimensions().hash(&mut h);
            ground.as_raw().hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    // The process-global zvm palette: packed Standard colours rasterise through
    // it (`standard_pixel_rgb` → `standard_true_colour`), so an
    // `InterpreterProfile` palette swap must rebuild the canvas.
    (zvm::screen::palette() as u8).hash(&mut h);
    // The machine pair and the RESOLVED host pair: the fallback ink/page every
    // packed colour resolves onto, which also folds in the theme's transcript
    // style and the terminal defaults.
    state.v6_page_pair.get().hash(&mut h);
    let (dfg, dbg) = v6_host_pair(state);
    (dfg.0, dbg.0).hash(&mut h);
    // The theme's ANSI palette, resolved to RGB: `resolve_zcolour` maps the
    // few Standard colours the fixed tables don't catch through it.
    for c in &state.colors.palette {
        crate::render::v6_layout::color_to_rgba(*c, image::Rgba([0, 0, 0, 255])).0.hash(&mut h);
    }
    // Live config the compute branches on.
    state.config.honor_game_colours.hash(&mut h);
    state.config.v6_pixel_lock.hash(&mut h);
    // Art density and the text metric (cell + advances), per CLAUDE.md's
    // art-vs-text density rule: both feed `FrameGeometry` and the canvases.
    state.v6_art_scale.hash(&mut h);
    state.v6_text.wrap_fingerprint().hash(&mut h);
    (state.v6_text.cell().w(), state.v6_text.cell().h()).hash(&mut h);
    // The band ground under an alpha-flattening backend: it is baked into every
    // shipped band, and on a replay frame the per-band content hashes are NOT
    // recomputed — so it must invalidate here.
    match backend_flattens_alpha_to_black(picker).then(|| v6_composite_page(Some(story), dbg, state)) {
        Some(p) => {
            1u8.hash(&mut h);
            p.0.hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    h.finish()
}

/// SQ-1187: the cached product of one hybrid-ring COMPUTE — everything the
/// draw half needs to replay the frame without rebuilding a canvas or running a
/// layout scan. Owned throughout (the window MODEL is deep-cloned fresh each
/// frame, so nothing here may borrow it); stored in
/// [`crate::render::graphics::GraphicsRender`]'s `hybrid` slot and keyed by
/// [`v6_hybrid_gen`].
pub(crate) struct HybridFrame {
    pub(crate) key: u64,
    /// The finished chrome canvas (art + rasterised non-glyph text + pages),
    /// the source every band crop and stretch reads.
    canvas: image::RgbaImage,
    /// The graphics-only canvas + story plate: the "is there art here?" oracle
    /// surface, still needed by `stamp_runs_over_art` at draw time.
    gfx: image::RgbaImage,
    scale: crate::render::v6_layout::Scale,
    menu: Option<crate::render::v6_layout::Scale>,
    plan_is_menu: bool,
    ring_plan: &'static str,
    ring_clip: Option<(u16, u16)>,
    image_scale: f32,
    lock_inapplicable: bool,
    lock_fallback: bool,
    viewport: Rect,
    vp_native: (u32, u32, u32, u32),
    strips: Vec<ChromeStrip>,
    menu_strips: Vec<ChromeStrip>,
    /// Art strips with real art behind them (`ChromeRowOracle::region_has_art`),
    /// keyed by rect — the draw half's replacement for the oracle's borrow.
    art_backed: std::collections::HashSet<(u16, u16, u16, u16)>,
    flank_borders: Vec<(Rect, Option<FlankBorderExt>, Option<FlankBorderExt>)>,
    divider_exts: Vec<FlankBorderExt>,
    flank_panels: Vec<(Rect, FlankPanel)>,
    tiled_flanks: Vec<(Rect, TiledFlank)>,
    tile_cols: u16,
    live: std::collections::HashSet<crate::render::graphics::BandKey>,
    packed_text: Vec<crate::render::graphics::PackedText>,
    over_art_runs: Vec<crate::engine::PxText>,
}

/// SQ-1187: one COMPUTE of the hybrid chrome ring — every layout decision,
/// canvas build and band classification the ring needs, packaged as an owned
/// value the draw half can replay on any later frame whose [`v6_hybrid_gen`]
/// key matches. Nothing in here may touch the terminal buffer, the graphics
/// renderer or any published `Cell` — those all belong to the draw half, which
/// runs every frame (cached or not).
#[allow(clippy::too_many_arguments)]
fn build_hybrid_frame(
    hkey: u64,
    layout: &crate::render::v6_layout::V6Layout<'_>,
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
    area: Rect,
    picker: &ratatui_image::picker::Picker,
    default_fg: image::Rgba<u8>,
    default_bg: image::Rgba<u8>,
    state: &AppState,
) -> HybridFrame {
    use crate::render::v6_layout as v6;
    let fs = picker.font_size();
    let cell_px = (fs.width, fs.height);
    let pane_dev = (
        area.width as u32 * fs.width.max(1) as u32,
        area.height as u32 * fs.height.max(1) as u32,
    );
    // SQ-0936: one global letterbox factor for the whole native
    // screen, quantized to the artwork's own ladder when the
    // player has asked for it (`v6_pixel_lock`, default off).
    // GLOBAL rather than per-picture, and Journey is what settles
    // that: its picture sits in its own window beside a drawn
    // divider rule, so a per-picture rung would stop the art short
    // of its own frame and open a gap. A pane too small for even
    // the smallest rung falls back to free scaling and says so as
    // a diagnostic, never on the game screen.
    //
    // SQ-0978: and the ladder is quantized in DEVICE pixels, which
    // is a unit half-blocks does not have — see
    // `crate::render::graphics::v6_pixel_lock_applies` for the
    // measurement. The lock is inert on that backend, reported as
    // inert, and never dressed up as a snap that happened.
    let lock_applies = crate::render::graphics::v6_pixel_lock_applies(picker);
    let lock_inapplicable = state.config.v6_pixel_lock && !lock_applies;
    let (scale_center, lock_fallback) =
        v6::FrameGeometry::new(native, state.v6_art_scale, state.v6_text.cell())
            .fitted_scale(pane_dev, state.config.v6_pixel_lock && lock_applies);
    // Publish the letterbox factor — the magnification the ART
    // is drawn at, and since SQ-1002 nothing else. The scale
    // FACTOR is unchanged by the SQ-0505 anchoring below (only
    // the vertical offset moves), so publish it now.
    //
    // Inline story pictures used to be scaled by it, to "match
    // the chrome ring". They must not: a drop-cap is drawn
    // INSIDE the text flow, and in hybrid the text is glyphs at
    // one native cell per terminal cell while the ring is pixels
    // at `s`. Matching the ring made Zork Zero's cap twice the
    // height of the paragraph it opens. `render_transcript` reads
    // the flag instead and sizes them at the TEXT's rate.
    let image_scale = scale_center.s;
    // SQ-0896: TWO art canvases, and the difference between them is
    // the whole of this quest.
    //
    // `frame_art` is the chrome-only artwork — the frame the game drew
    // AROUND its story window — and it is the oracle the inset must
    // use: `story_clear_native` walks the window's edges in until none
    // of them touches an opaque pixel, and measuring that against
    // anything carrying the story's own plate would inset the window
    // out of its own backdrop (fmvpoker's hollow table comes back
    // width 0). Rasterised glyphs are excluded for the same reason
    // they always were (SQ-0500/0728): opaque is not artwork.
    //
    // `gfx` is that PLUS the story window's own plate, and it is what
    // every downstream stage asks "is there art here?" of. It has to
    // carry the plate, because the bands that now cover the plate exist
    // only by the viewport having given those cells up — and an Art
    // strip whose oracle says there is nothing behind it is skipped and
    // never drawn (`strip_has_art`). Without this the ring would lose
    // the prose area AND still draw no picture.
    let frame_art = v6::build_graphics_canvas(&layout.chrome, native);
    // Step (b) of the user's ordering, for the region the ring never
    // owned: the story viewport is cut from what the ART leaves, not
    // from the raw window box. `None` = the plate owns the screen and
    // no prose belongs on this frame (SQ-0707), which for the ring
    // means the whole pane is chrome.
    let vp_native = v6::story_text_native(Some(story), &frame_art, layout.story_gfx, state.v6_text.cell());
    let plate_owns_screen = vp_native.is_none();
    // Fall back to the declared box so every stage below still has a
    // native rectangle to reason about; the empty viewport is applied
    // once, after the plans and the overlay push, so a plan cannot
    // divide by a zero-height story region on the way there.
    let vp_native = vp_native.unwrap_or((
        story.x_px as u32,
        story.y_px as u32,
        story.w_px as u32,
        story.h_px as u32,
    ));
    let mut gfx = frame_art;
    v6::blit_story_gfx(&mut gfx, layout.story_gfx);
    let gfx = gfx;
    let chrome_runs: Vec<&crate::engine::PxText> = paint_runs(&layout.chrome).collect();
    // SQ-0505 dynamic hybrid layout: reclaim the letterbox dead
    // space below the story when the bottom edge is text-only
    // (Journey's command menu) or empty (Arthur — header art +
    // side borders, open below). A game whose frame encloses the
    // story to the native bottom (Zork0) keeps today's centred
    // letterbox. `slack` is the vertical letterbox margin in
    // device pixels (zero when the pane is at/below the scaled
    // native height — nothing to reclaim, degrade to centred).
    let scaled_h = (native.1 as f32 * scale_center.s).round() as u32;
    let slack = pane_dev.1.saturating_sub(scaled_h);
    let plan = hybrid_bottom_plan(story, &gfx, &layout.chrome, native, slack, state.v6_text.cell());
    let reclaim = !matches!(plan, BottomPlan::Letterbox);
    // Resolve the story scale, the story viewport, and an
    // optional bottom-anchored menu scale.
    //   Letterbox → centred (today's behaviour, unchanged).
    //   Extend    → top-anchor (off_y = 0), story grows to the
    //               pane bottom; flanks below the side art blank.
    //   Menu      → top-anchor the story + chrome, bottom-anchor
    //               the command strip to the pane bottom, story
    //               fills between at constant width.
    let top_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: 0 };
    let (scale, viewport, menu) = match plan {
        BottomPlan::Letterbox => {
            let vp = v6::native_viewport_box(Some(vp_native), &scale_center, (area.width, area.height), cell_px);
            (scale_center, Rect::new(area.x + vp.x, area.y + vp.y, vp.width, vp.height), None)
        }
        // Extend (Arthur) and Frame (Zork0/Shogun) top-anchor the
        // story and grow it to the pane bottom identically; they
        // differ only in how the flanks below the side art are
        // treated — Extend blanks them, Frame stretches them (the
        // reclaim block below branches on the plan).
        BottomPlan::Extend | BottomPlan::Frame => {
            let vp = v6::native_viewport_box(Some(vp_native), &top_scale, (area.width, area.height), cell_px);
            let (x, y) = (area.x + vp.x, area.y + vp.y);
            // SQ-1008: the reclaim takes the letterbox slack below the
            // story — and it may not take a row the GAME is still using.
            //
            // Both arms reach here on the premise that nothing lives
            // below the story window: `Extend`'s by elimination, and
            // this pair's enclosed-frame arm by the story reaching
            // "within one native text row of the screen bottom". That
            // second premise is one row loose, and Arthur spends the row
            // it forgives. `Arthur - The Quest for Excalibur.adf`
            // (release 54 / serial 890606) answers `hint` in play by
            // laying window 3 across native `(28, 384, 584, 16)` — the
            // LAST text row of a 640x400 screen, with window 0 ending at
            // 384 — and printing *"If only you had a crystal ball...."*
            // into it. `menu_strip_below_story` never sees it, because
            // its own `native.1 <= story_bottom + cell.h` guard returns
            // false before it looks; so the plan is `Frame` (his poles
            // flank the story full height), the viewport grew to
            // `area.bottom()`, and `content_ring_bands` carved
            // `pane − viewport` and found no bottom band to put the box
            // in. MEASURED, same frame, sixteen turns in: viewport 11
            // rows at 80x25 and 100x25 with the box drawn on row 24;
            // 17 rows at 100x34, 21 at 80x34, 35 at 80x48 with the box
            // drawn nowhere. Any v6 window below the story window was
            // invisible at any terminal taller than the game's own
            // screen, which is most terminals.
            //
            // The extra rows are not the defect and are not clamped
            // here: they are the transcript's, and window 0 is a
            // SCROLLING buffer whose history the player reads far past
            // its eleven native rows. What they may not include is the
            // game's own bottom-anchored content. So take the rows that
            // content needs off the pane's bottom and hand back the
            // bottom-anchored scale for them — which is exactly what the
            // `Menu` arm below does for Journey's command strip, reused
            // rather than restated so the two cannot drift. `rows` is 0
            // on every frame with nothing below the story, which is
            // every frame in the corpus but this one, and the arm is
            // then byte-identical to what it was.
            let rows = menu_band_rows(&menu_band_runs(&chrome_runs, story), state.v6_text.cell());
            if rows == 0 {
                (top_scale, Rect::new(x, y, vp.width, area.bottom().saturating_sub(y)), None)
            } else {
                let bottom_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: slack };
                let band_top = area.bottom().saturating_sub(rows).clamp(y + 1, area.bottom());
                (top_scale, Rect::new(x, y, vp.width, band_top.saturating_sub(y)), Some(bottom_scale))
            }
        }
        BottomPlan::Menu => {
            let menu_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: slack };
            let vp = v6::native_viewport_box(Some(vp_native), &top_scale, (area.width, area.height), cell_px);
            let (x, y) = (area.x + vp.x, area.y + vp.y);
            // SQ-0765: the MENU is the fixed-height window here, and the
            // art and the story take what is left above it — the
            // inverse of every other v6 title, where the fixed window is
            // the status band at the top. So the band's height is the
            // menu's OWN height and nothing else's leftover:
            // [`menu_band_rows`] counts the game text rows it carries,
            // because hybrid draws chrome text one game row per terminal
            // row (SQ-0543), and the band is bottom-anchored to that.
            //
            // It used to be the other way round. `menu_top` was the
            // first terminal row carrying a menu run through the
            // bottom-anchored menu SCALE, so the story viewport was the
            // scale-derived quantity and the band was the remainder —
            // measured off the user's own dumps, 9 rows at pane height
            // 61/scale 1.43 and 11 at 61/1.96, for a menu whose content
            // stayed a constant 7 game rows. The rows the content never
            // reached were painted by nothing, which put the frame's own
            // bottom border three rows above the pane's last row
            // (SQ-0754), and at a short pane the reverse: the band came
            // out SHORTER than its content and clipped the last menu
            // line off the screen entirely.
            //
            // SQ-0548 (a run-less leftover row inside the band redrawing
            // a squashed slice of the frame's bottom edge) cannot recur
            // here by construction: the band is exactly the menu's own
            // rows, and `menu_band_strips` gives it to the cell path
            // whole, so there is no leftover row to misclassify.
            let menu_rows = menu_band_rows(&menu_band_runs(&chrome_runs, story), state.v6_text.cell());
            let menu_top = area.bottom().saturating_sub(menu_rows).clamp(y + 1, area.bottom());
            (top_scale, Rect::new(x, y, vp.width, menu_top.saturating_sub(y)), Some(menu_scale))
        }
    };
    // SQ-0582: a status bar the game OVERLAYS on its story window
    // (advent.z6) leaves no chrome ring to carry it. Reserve its
    // rows off the top of the story viewport, so the band below
    // decomposes it exactly like a game that reserved the space
    // itself — a solid full-width Text strip, with the transcript
    // starting under it instead of scrolling through it. Measured
    // from the strip's own runs, not its declared height: a 20px
    // window is 1.25 cells tall but carries a single text row.
    let overlay_strip = overlaid_status_strip(&layout.chrome, story, native.0, state.v6_text.cell());
    // The overlaid strip's native bottom, so `decompose_chrome_strips`
    // counts its runs as band content: they sit INSIDE the story box,
    // which its above/below test rejects.
    let overlay_bottom =
        overlay_strip.map(|s| s.y_px.saturating_add(s.h_px) as i32).unwrap_or(0);
    let viewport = match overlay_strip {
        Some(strip) => {
            let last = match &strip.node {
                WinNode::Grid(g) => {
                    let bound = strip_rows(strip, g, state.v6_text.cell());
                    g.px_texts
                        .iter()
                        .filter(|t| !t.text.trim().is_empty())
                        .filter(|t| {
                            bound.is_some_and(|b| state.v6_text.cell().row_of(t.y) <= b)
                        })
                        .map(|t| run_cell(t, &scale, cell_px, area, state.v6_text.cell()).1)
                        .max()
                }
                _ => None,
            };
            match last {
                Some(r) if r >= area.y as i32 => {
                    let top = (r as u16).saturating_add(1).min(area.bottom());
                    let y = viewport.y.max(top);
                    Rect::new(viewport.x, y, viewport.width, viewport.bottom().saturating_sub(y))
                }
                _ => viewport,
            }
        }
        None => viewport,
    };
    // SQ-0896: the plate owns the screen — no prose box survives it, so
    // there is no story region at all and the ring gets the whole pane.
    // An empty viewport is not a special case for anything downstream:
    // `content_ring_bands` carves `pane − viewport` and a zero-height
    // viewport makes that the pane, which then decomposes into Art and
    // Text strips by exactly the rules a top band already uses. The one
    // thing that must not happen is rendering a transcript into it; see
    // the guard on `render_node` below, which mirrors raster returning
    // no scroll metrics for the same frame.
    let viewport = if plate_owns_screen {
        Rect::new(area.x, area.y, area.width, 0)
    } else {
        viewport
    };
    // SQ-0500: a full-width chrome band (top/bottom) is carved
    // into horizontal strips — an ART strip (opaque frame
    // graphics behind it) keeps the scaled pixel RING; a
    // TEXT-ONLY strip (no graphics behind, just status/menu
    // runs) paints as crisp terminal CELLS. Journey's bottom
    // command menu becomes text while its left picture column
    // (a narrow side band) stays ring; Arthur's status row
    // becomes text between the art panel above and the story
    // below; Zork0's status sits ON banner art so every strip
    // stays ring. The graphics-only canvas answers "art behind
    // this strip?" — the full chrome canvas can't, since its
    // rasterized text is itself opaque.
    // Cell rects of the secondary prose windows: the ring leaves
    // those rows to them (SQ-0585). Computed HERE rather than just
    // before `decompose_chrome_strips` because the content-built ring
    // needs them too — a flank must not extend across a row a prose
    // panel draws itself.
    let panel_rects: Vec<Rect> = layout
        .chrome
        .iter()
        .filter(|pw| matches!(&pw.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
        .map(|pw| px_rect_to_cells(pw, &scale, cell_px, area, 0))
        .collect();
    // SQ-0894, step (b) becoming live: the FLANK claims its columns
    // first and the text region is what is left. A flank is a column of
    // the frame's own side artwork at the pane's edge whether or not the
    // story window happened to leave room for it, so where the art runs
    // past the story box the box gives those columns up.
    //
    // Not via `story_clear_native`, which is the other half of step (b)
    // and measures OVERLAP: on the frame that motivates this the art and
    // the story box are ADJACENT, not overlapping (Shogun's Amiga credits
    // screen paints its ornaments down to native y=335 and puts window 0
    // at y=336), so the overlap test correctly finds nothing to inset and
    // the columns still have to come from somewhere. They come from here.
    let (art_left, art_right) = flank_art_columns(&gfx, &scale, cell_px, area, state.v6_text.cell());
    let viewport = {
        let x = viewport.x.max(art_left).min(viewport.right());
        let r = viewport.right().min(art_right).max(x);
        if r > x {
            Rect::new(x, viewport.y, r - x, viewport.height)
        } else {
            viewport
        }
    };
    // Which native rows are BARS the game draws edge to edge (SQ-0515).
    // Only the keys are read here, so the style arguments are
    // immaterial — the styles themselves belong to the cell path, which
    // resolves them against its own theme base.
    let bar_rows: std::collections::HashSet<u16> = full_width_flood_rows(
        &layout.chrome,
        native.0,
        ratatui::style::Style::default(),
        TextInk::of(state),
        state.v6_text.cell(),
    )
    .into_keys()
    .collect();
    // …and how far the chrome text on each native row can REACH: the
    // native columns its own windows span (SQ-0949). A ribbon is as
    // wide as its window and no wider, and the flank veto has to know
    // that — Arthur's status window is 584 of 640 native columns, so it
    // reads as a bar and still stops 28 columns short of each edge,
    // which is exactly where his poles stand.
    let row_spans: std::collections::HashMap<u16, (u16, u16)> = {
        let mut m: std::collections::HashMap<u16, (u16, u16)> = Default::default();
        for pw in &layout.chrome {
            let WinNode::Grid(g) = &pw.node else { continue };
            let span = (pw.x_px, pw.x_px.saturating_add(pw.w_px));
            for t in &g.px_texts {
                let row = state.v6_text.cell().row_of(t.y);
                let e = m.entry(row).or_insert(span);
                e.0 = e.0.min(span.0);
                e.1 = e.1.max(span.1);
            }
        }
        m
    };
    // SQ-0894: the ring is carved by what the chrome CONTAINS, not as
    // the story viewport's complement, so each flank is one column over
    // every contiguous row of art it may own.
    let row_oracle = ChromeRowOracle {
        v6_cell: state.v6_text.cell(),
        pane: area,
        scale: &scale,
        cell_px,
        story_native: vp_native,
        overlay_bottom,
        panels: &panel_rects,
        gfx: &gfx,
        runs: &chrome_runs,
        bar_rows: &bar_rows,
        row_spans: &row_spans,
    };
    let bands = content_ring_bands(area, viewport, menu.is_some(), &row_oracle);
    // SQ-0505: in the Menu plan the bottom band IS the command
    // strip — decompose it through the bottom-anchored `menu`
    // scale, and the top+side ring bands through the story
    // `scale`. Each strip is later drawn through the scale it was
    // classified with, so the menu lands at the pane bottom while
    // the story/top/sides stay top-anchored.
    let mut ring_bands = bands;
    let menu_bands: Vec<Rect> = if menu.is_some() {
        // The menu band IS the bottom band — asked by role, not by
        // recognising its shape (SQ-0894). The old test was
        // `width == area.width && y == viewport.bottom()`, which is a
        // description of where `chrome_bands` happened to put it.
        let m: Vec<Rect> = ring_bands
            .iter()
            .filter(|(role, _)| *role == v6::BandRole::Bottom)
            .map(|(_, r)| *r)
            .collect();
        ring_bands.retain(|(role, _)| *role != v6::BandRole::Bottom);
        m
    } else {
        Vec::new()
    };
    // SQ-0511: the Frame/Menu plans STRETCH the side flank bands to
    // fill the reclaimed space (drawn below); the whole flank band
    // survives here so the stretch has a full-height target. The
    // Extend plan (Arthur) instead CLIPS the ring bands to the chrome
    // art's actual vertical extent (its lowest opaque native row,
    // mapped through the story scale) so the flanks BELOW its side
    // art stay the theme backdrop — no art stretching or tiling there.
    // Letterbox is untouched (its bands lie within the scaled canvas).
    let stretch_flanks = matches!(plan, BottomPlan::Frame | BottomPlan::Menu);
    let ring_plan = match plan {
        BottomPlan::Letterbox => "letterbox",
        BottomPlan::Extend => "extend",
        BottomPlan::Frame => "frame",
        BottomPlan::Menu => "menu",
    };
    let mut ring_clip: Option<(u16, u16)> = None;
    if reclaim && !stretch_flanks {
        let ch = cell_px.1.max(1) as f32;
        let art_bottom_px =
            (0..gfx.height()).rev().find(|&y| (0..gfx.width()).any(|x| gfx.get_pixel(x, y)[3] >= 128));
        let clip_row = match art_bottom_px {
            Some(y) => area.y + ((scale.off_y as f32 + (y + 1) as f32 * scale.s) / ch).ceil() as u16,
            None => area.y,
        };
        // SQ-0571: the clip must never guillotine a chrome TEXT row
        // that sits between the art and the story — Arthur's status
        // bar. The clip rounds the art's native bottom UP through the
        // scale; `run_cell` maps a run's native top by ROUNDing. Both
        // read the same native boundary (Arthur's art ends at 192, its
        // status row starts at 192), so whenever `192·s/cell_h` has a
        // fraction >= 0.5 the two agree and the clip lands exactly ON
        // the status row, evicting it from the band. With no Text strip
        // covering it the run is never cleared from the band canvas
        // (`clear_text_rows` below), so the status painted as a squashed
        // raster slice of the frame instead of crisp cells — the
        // width-dependent "corrupted location bar" (broken at 96..=99
        // columns on an 8x17 cell, clean at 95 and 100).
        //
        // Raise the clip past the LAST pure-text chrome row above the
        // story. Deliberately only text rows: a run-less row below the
        // art still gets clipped (it would otherwise coalesce into an
        // Art strip and redraw a squashed slice of the frame's edge,
        // the SQ-0548 defect), and a run OVER art is already ring
        // content that the unraised clip places correctly.
        let story_top = story.y_px as i32;
        // SQ-1020: `over_art` is the oracle's own predicate and
        // this used to be a second copy of it — same question,
        // written twice, and only one of them was converted when
        // the Version 6 cell stopped being 8x16 everywhere. The
        // copy passed `run_px(...)` for the width and a bare `16`
        // for the height, in the SAME call. One authority now.
        let text_above = chrome_runs
            .iter()
            .filter(|t| {
                !t.text.trim().is_empty()
                    && state.v6_text.cell().bottom_px(t.y) as i32 <= story_top
                    && !row_oracle.over_art(t)
            })
            .map(|t| run_cell(t, &scale, cell_px, area, state.v6_text.cell()).1)
            .max();
        let clip_row = match text_above {
            Some(r) if r >= 0 => clip_row.max((r as u16).saturating_add(1)),
            _ => clip_row,
        };
        // SQ-0582: never clip above the story viewport. The rule
        // above only spares text that sits above the story WINDOW,
        // so a bar the game overlays on the story instead (advent.z6)
        // matched neither test — with no art either, the clip landed
        // at the pane top and dropped the very band the inset above
        // just reserved for it. Whatever is above the viewport is
        // chrome by construction; it survives.
        let clip_row = clip_row.max(viewport.y);
        // Record what clipped the ring (SQ-0587). Arthur's side
        // borders live in the flank bands, and this clip is what
        // drops them: it trims the ring to the graphics canvas's
        // lowest opaque row, so a canvas that lost its lower art
        // takes the side borders with it.
        ring_clip = Some((
            art_bottom_px.map(|y| y as u16).unwrap_or(u16::MAX),
            clip_row,
        ));
        // SQ-0698: …and a flank whose art we know how to EXTEND is
        // spared that trim. The clip is what dropped Arthur's side
        // poles at the row his artwork happens to stop — native 379
        // of 400, terminal row 31 of a 64-row pane — leaving the
        // frame open down its whole lower half. Tiling gives that
        // band something to draw all the way to the story viewport's
        // bottom, so the band must survive to be drawn. Reserved to a
        // RECOGNISED flank (`v6_border::recognize`): a game with no
        // side art, or side art of a shape this code does not know,
        // is clipped exactly as before.
        for (role, b) in &mut ring_bands {
            // Asked by role (SQ-0894): `b.width < area.width` meant
            // "a flank" only while the top and bottom bands spanned
            // the pane.
            //
            // SQ-0894 measured this exemption for removal — §4 of the
            // pipeline document calls it "a patch on a patch" — and
            // KEPT it. Deleting it fails
            // `arthur_hybrid_tall_pane_extends_story_to_bottom`: the
            // clip trims the ring to the art's lowest opaque row, and
            // a content-built flank is still a band the clip can cut.
            // Owning the right ROWS does not exempt a flank from a
            // later stage shortening it; that is a separate decision
            // and it still has to be made here.
            if role.is_flank() && flank_border_art(*b, area, &scale, cell_px, native, &gfx).is_some() {
                continue;
            }
            if b.y >= clip_row {
                b.height = 0;
            } else {
                b.height = b.height.min(clip_row - b.y);
            }
        }
        ring_bands.retain(|(_, b)| b.height > 0 && b.width > 0);
    }
    // SQ-0944: `over_art_runs` is the text the game printed ON its
    // artwork. Empty unless this backend can show a glyph in a cell
    // an image covers — see `backend_layers_glyphs_over_art`, which
    // is the whole gate: everywhere else these runs stay pixels and
    // every line below this one behaves exactly as it did.
    let (mut strips, over_art_runs) = decompose_chrome_strips(&ring_bands, &row_oracle);
    let over_art_runs: Vec<crate::engine::PxText> = if backend_layers_glyphs_over_art(picker) {
        over_art_runs.into_iter().cloned().collect()
    } else {
        Vec::new()
    };
    // An ART strip with no actual art behind it draws a rasterized
    // slice of the chrome canvas — which carries TEXT too, so on a
    // text-only v6 story (advent) that is pure noise painted over the
    // pane. Under a graphics protocol the image composites ABOVE the
    // cells, so it cannot even be overdrawn. Skip those, and let
    // `/dump-windows` say which ones were skipped. (SQ-0585)
    // SQ-0750: the question is whether art lies behind THIS STRIP, so
    // the test is its own native REGION — both axes. It used to ask
    // only about the strip's rows, across the canvas's whole width,
    // which is the same question for a full-width top/bottom band and
    // a different one for a side flank: Journey's right-hand flank is
    // eight native pixels of frame border with no artwork in it at
    // all, and it was drawn as a band anyway because the LEFT flank's
    // picture shares its rows. That band is a bitmap of a `│` the game
    // printed as a character — 16x900 px per frame to draw one rule.
    // Classify a strip by what is in it, not by where it sits.
    // …and it asks the ORACLE, rather than carrying its own copy
    // of the inverse (SQ-1059). `ChromeRowOracle::region_has_art`
    // is that same mapping, built from the same `area`, `scale`,
    // `cell_px` and `gfx` this closure captured — and its doc has
    // always claimed to be "shared with the caller's art test so
    // the two do not drift", which was false in the commit that
    // wrote it: SQ-0894 pasted this body into the oracle instead
    // of calling it, in a commit whose own message said "the fix
    // should not add a fourth instance of it". Nothing in the
    // language kept the two equal; they simply had not diverged
    // yet, which is what SQ-1020 is in this same file.
    let strip_has_art = |r: &Rect| -> bool { row_oracle.region_has_art(*r) };
    // SQ-0894 MEASURED THIS FOR REMOVAL AND KEPT IT. The
    // content-built ring was expected to subsume the walk: a flank
    // now owns its rows by what is in its own columns, so the
    // remainder row beside the viewport should need no handing back.
    // For every flank whose border is ART that is exactly what
    // happens, and the walk no longer fires on Zork Zero, Arthur or
    // Shogun.
    //
    // It is still load-bearing for ONE case, and deleting it fails
    // `journeys_frame_side_rules_survive_a_pane_with_no_letterbox_slack`
    // with the original symptom — Journey's Amiga press at a 96x26
    // pane, "row 1 of the rule's span (1, 2, 2, 17) holds '│' in 0
    // column(s), not 1". The reason is that Journey's side rule is a
    // CHARACTER the game printed, not artwork: the graphics-only
    // canvas is empty across those columns by construction (that is
    // the whole of SQ-0750), so the ring's art test cannot see the
    // border and the flank declines the remainder row. The walk
    // reaches it by geometry instead, which is the one thing content
    // classification cannot do for content that is not pixels.
    //
    // So it stays, narrowed in purpose rather than in code. Removing
    // it needs the flank's row test to accept the game's own border
    // GLYPHS alongside art — a change to the glyph-border machinery,
    // not to this walk.
    // SQ-0747: the QUANTIZATION REMAINDER beside the story viewport belongs
    // to the flanks, not to the full-width band and not to nothing —
    // ABOVE the viewport and BELOW it alike.
    //
    // `story_viewport_box` quantizes the story's top edge OUTWARD to a
    // whole cell, while the top band runs down to that quantized row. So
    // a terminal row can fall between the frame's top rule and the first
    // prose row whose own native span is already INSIDE the story box —
    // it is the half-cell the viewport rounded away, not chrome. The
    // full-width band draws it either way, and both outcomes are wrong:
    // with nothing behind it the row classifies Empty → Art → skipped and
    // is never written at all (terminal row 2 of the captured 115-column
    // frame, across every column: a bare stripe through the picture panel
    // and a one-row hole where the frame's top rule meets its two side
    // rules), and with the picture's first pixels behind it the band
    // paints a one-row squashed slice of the WHOLE canvas across the pane
    // — the picture's top standing above its own panel, which is the other
    // half of this quest.
    //
    // The flanks own those columns and have real content in them (their
    // borders, their panel ground), so the strip goes to them; the story's
    // own columns keep the story's ground, which is what stands beside the
    // prose one row lower.
    //
    // Bounded by CONTENT, and WHOLE STRIPS only. A band that carries the
    // game's chrome art down to the viewport — Zork Zero's and Shogun's
    // banners, Arthur's header — is one tall strip whose first row is
    // above the story box, so neither test below holds for it and it is
    // untouched. Trimming a row off such a strip instead of leaving it
    // alone would take a row of banner away, which is why this walks
    // strips rather than rows.
    //
    // SQ-0747, second pass: and the story box has TWO quantized edges, so
    // there is a remainder under it as well. `story_viewport_box` rounds
    // the bottom in too, and the row it rounds away is a full-width band
    // exactly like the one above — measured off `Journey - The Quest
    // Begins.adf` (release 30 / serial 890322) at a 121x36 terminal: the
    // picture ran to row 23, the menu began at row 25, and between them a
    // 119-column one-row band painted a squashed slice of the whole canvas
    // straight across BOTH of the frame's side rules. At a 236x68 terminal
    // the same row has no art behind it, classifies skipped, and reaches
    // the screen unwritten — the two halves of the very same defect the
    // walk above already knows by name. One rule, expressed once: a
    // full-width Art strip that is the remainder of the picture's own box
    // belongs to the flanks, whichever side of the viewport it falls.
    let flank_at = |strips: &[ChromeStrip], edge: u16, top: bool| {
        strips.iter().any(
            |s| matches!(s, ChromeStrip::Art(role, r) if role.is_flank() && (if top { r.y } else { r.bottom() }) == edge),
        )
    };
    {
        let ch = cell_px.1.max(1) as f32;
        let sc = scale.s.max(0.001);
        let inv_y = |row: u16| ((row.saturating_sub(area.y)) as f32 * ch - scale.off_y as f32) / sc;
        // SQ-0896: the walk is about the QUANTIZATION REMAINDER of the
        // rect the viewport was cut from, so it reads `vp_native` and
        // not the declared window box. On every corpus frame the two
        // are the same rectangle; where a plate or frame art has moved
        // the viewport, the remainder is beside the viewport's own edge
        // and nowhere else.
        let story_bottom = (vp_native.1 + vp_native.3) as f32;
        let mut gap_top = viewport.y;
        if flank_at(&strips, viewport.y, true) {
            while gap_top > area.y {
                let Some(i) = strips.iter().position(|s| {
                    matches!(s, ChromeStrip::Art(role, r) if !role.is_flank() && r.bottom() == gap_top)
                }) else {
                    break;
                };
                let ChromeStrip::Art(_, r) = strips[i] else { unreachable!() };
                // Either half of the remainder qualifies, and which one it is
                // moves with the pane by fractions of a cell: the strip's own
                // native span is already inside the story box (so the band
                // would paint a squashed slice of the whole canvas over the
                // panel), or the band draws nothing there at all (so the rows
                // reach the screen unwritten). Anything else is real chrome and
                // stops the walk, as does a TEXT strip, whose runs are the
                // game's own.
                let remainder = (inv_y(r.y) + inv_y(r.y + 1)) / 2.0 >= vp_native.1 as f32;
                if !remainder && strip_has_art(&r) {
                    break;
                }
                strips.remove(i);
                gap_top = r.y;
            }
            if gap_top < viewport.y {
                for s in &mut strips {
                    if let ChromeStrip::Art(_, r) = s {
                        if r.width < area.width && r.y == viewport.y {
                            r.height += viewport.y - gap_top;
                            r.y = gap_top;
                        }
                    }
                }
            }
        }
        // …and downward, by the same test read from the other end. The
        // strip's LAST row is what is asked about here, so a tall band
        // carrying the game's own chrome (a menu's art, a bottom rule)
        // cannot be swallowed by having merely begun inside the box.
        let mut gap_bottom = viewport.bottom();
        if flank_at(&strips, viewport.bottom(), false) {
            while gap_bottom < area.bottom() {
                let Some(i) = strips.iter().position(|s| {
                    matches!(s, ChromeStrip::Art(role, r) if !role.is_flank() && r.y == gap_bottom)
                }) else {
                    break;
                };
                let ChromeStrip::Art(_, r) = strips[i] else { unreachable!() };
                let remainder = (inv_y(r.bottom() - 1) + inv_y(r.bottom())) / 2.0 <= story_bottom;
                // Bounded by CONTENT on both counts, and the second one is
                // what keeps the corpus still. Above the story a band that
                // carries the game's chrome is a TALL strip and fails the
                // remainder test on its own; below it, a game whose frame
                // closes along the pane's last row draws that row INSIDE its
                // own story box — Zork Zero's does, at 236x68 and 121x36 —
                // so the remainder test alone would swallow the bottom of its
                // frame into two flanks that only cover the sides. Ask what
                // is BETWEEN the flanks: the columns the band alone would
                // draw. Artwork there is the game's chrome and stops the
                // walk; nothing there means the band is drawing a squashed
                // slice of the story's own ground, or nothing at all.
                let middle = Rect::new(viewport.x, r.y, viewport.width, r.height);
                if !remainder || strip_has_art(&middle) {
                    break;
                }
                strips.remove(i);
                gap_bottom = r.bottom();
            }
            if gap_bottom > viewport.bottom() {
                for s in &mut strips {
                    if let ChromeStrip::Art(_, r) = s {
                        if r.width < area.width && r.bottom() == viewport.bottom() {
                            r.height += gap_bottom - viewport.bottom();
                        }
                    }
                }
            }
        }
    }
    let menu_strips = match &menu {
        Some(_) => menu_band_strips(&menu_bands, story, &chrome_runs),
        None => Vec::new(),
    };
    // SQ-0504: rows drawn as terminal CELLS (pure-text strips)
    // must not ALSO reach the pixel bands. Carve every text-strip
    // run's native rows out of the band canvas: excludes the
    // rasterized menu/status from every uploaded band image (a
    // sub-cell letterbox boundary otherwise bleeds the raster bar
    // behind the cells) and decouples each art band's hash from
    // the menu text (navigating the menu re-encodes only changed
    // art, not every band). Beside-story runs — Journey's vertical
    // picture/text divider — are NOT text strips, so they stay in
    // the side band's ring untouched.
    // SQ-0894: over the strip's OWN native columns. A strip that spans
    // the pane still carves the whole row (every strip on the corpus
    // does, so this is byte-identical there); one a flank has narrowed
    // must not erase the flank's source pixels beside it.
    let strip_native_cols = |r: Rect| -> (u32, u32) {
        if r.x <= area.x && r.right() >= area.right() {
            return (0, u32::MAX);
        }
        let inv = |c: u16| {
            ((c.saturating_sub(area.x) as f32 * cell_px.0.max(1) as f32
                - scale.off_x as f32)
                / scale.s.max(0.001))
            .max(0.0) as u32
        };
        (inv(r.x), inv(r.right()))
    };
    let text_run_tops: Vec<(u16, u32, u32)> = strips
        .iter()
        .chain(menu_strips.iter())
        .flat_map(|s| match s {
            ChromeStrip::Text(r, runs) => {
                let (x0, x1) = strip_native_cols(*r);
                runs.iter().map(|t| (t.y.max(1) - 1, x0, x1)).collect::<Vec<_>>()
            }
            ChromeStrip::Art(..) => Vec::new(),
        })
        .collect();
    // ── the chrome CANVAS, built here rather than 650 lines up ──
    //
    // SQ-0903. Every classification above — the viewport, the
    // strips, the ring bands, which rows the ring will draw as
    // GLYPHS — is computed from `layout` and the ART canvases and
    // never reads this one. That was not obvious while the canvas
    // was built first: it looked like an ordering constraint, and
    // the rasterise-then-carve sequence below looked like the price
    // of one. It is not. Only seven statements in the 700 lines
    // between the old site and the first read touched `canvas`, and
    // all seven are these.
    //
    // Building it here means `text_run_tops` — the rows the ring has
    // just decided to draw with glyphs — is known BEFORE a pixel is
    // rasterised, so those rows are never painted instead of being
    // painted and then carved back out.
    // SQ-0903: the rows the ring has just decided to draw with
    // GLYPHS. `text_run_tops` is that decision, taken a few lines
    // up and reaching the canvas builder before it paints rather
    // than reaching a carve afterwards.
    // SQ-0934: the chrome the CANVAS is built from excludes the
    // promoted story grid — the menu the game printed into the
    // ring's clear middle after withdrawing its buffer.
    //
    // It is in `chrome` because its runs must reach `chrome_runs`,
    // but it is the STORY SURFACE, and rasterising a story surface
    // is the thing hybrid exists not to do. Leaving it in put the
    // menu's pixels — its text AND the reverse-video bar under the
    // selected item — into the canvas the ring bands are cropped
    // from, so fragments of the menu came back out inside the ring:
    // a sliver of the first text column showing at the flank's inner
    // edge, and the selection bar's two ends appearing IN the left
    // and right flanks, level with the selection and then again
    // wherever a flank band tiles (SQ-0894). "Mirrored top and
    // bottom, and overrunning the frame", as reported, is a tiled
    // repeat of a strip that should never have carried text.
    //
    // The middle is drawn as terminal cells by `render_node` above,
    // which is the whole point of hybrid; the canvas only has to
    // carry what the bands ship, and the bands are the frame.
    let ring_chrome: Vec<&crate::engine::PositionedWindow> = layout
        .chrome
        .iter()
        .copied()
        .filter(|it| !layout.story.is_some_and(|st| std::ptr::eq(*it, st)))
        .collect();
    let mut glyph_rows: std::collections::HashSet<u16> =
        text_run_tops.iter().map(|&(top, _, _)| top).collect();
    // SQ-0944: …and the rows of text the game printed ON its
    // artwork, when this backend can draw a glyph in a cell the
    // art covers. `over_art_runs` is already empty on every
    // backend that cannot, so this adds nothing there and the
    // canvas keeps rasterising them exactly as before.
    //
    // Pass 1 of `build_chrome_canvas` — the ARTWORK — is never
    // skipped, so the band still ships the picture these glyphs
    // will sit on. Only the TEXT layer stops being painted, which
    // is what stops the crisp glyph landing on a blurred copy of
    // itself and, worse, what stops the ground sampled from under
    // it being a sample of the rasterised text.
    glyph_rows.extend(over_art_runs.iter().map(|t| t.y.max(1) - 1));

    // SQ-0937: …and the rows the ring is about to stamp INSIDE the
    // story box, which are glyph rows for exactly the same reason and
    // had never been counted as such.
    //
    // `text_run_tops` collects from the ring's STRIPS, and the story
    // box is not a strip — so a chrome run landing in the box was
    // rasterised into the canvas AND stamped as a glyph a few hundred
    // lines below. Both, one over the other, and the rasterised copy
    // then travelled wherever the bands crop and tile from this canvas.
    //
    // The Macintosh press is where it shows, because it is the release
    // that KEEPS its primary buffer for the hint screen: its menu is
    // chrome runs inside the box, so it takes this path, while Blorb
    // and Amiga withdraw the buffer and have their menu grid promoted
    // to the story surface instead (SQ-0934), which is excluded from
    // this canvas outright.
    //
    // Same predicate as the packing below, deliberately — if the two
    // ever disagree about which runs are stamped, the difference is
    // drawn twice or not at all.
    for it in &ring_chrome {
        let WinNode::Grid(g) = &it.node else { continue };
        for t in &g.px_texts {
            let px = u32::from(t.x.max(1)) - 1;
            let py = u32::from(t.y.max(1)) - 1;
            if px >= vp_native.0
                && px < vp_native.0 + vp_native.2
                && py >= vp_native.1
                && py < vp_native.1 + vp_native.3
            {
                glyph_rows.insert(t.y.max(1) - 1);
            }
        }
    }
    let mut canvas = v6::build_chrome_canvas(&ring_chrome, native, default_fg, default_bg, &state.colors, v6::TextLayer::SkipGlyphRows(&glyph_rows), &state.v6_text);
    // SQ-0896: …and the STORY window's own plate, which the chrome
    // canvas excludes by construction — `classify_windows` sets a
    // `win == 0` Graphics aside as `story_gfx` so the ring does not
    // carry it. That was right while the ring could only draw outside
    // the story window; now the viewport is cut from what the art
    // leaves, the ring covers the plate's cells and needs its pixels
    // to crop. Blitted with the chrome ART layer, before the painted
    // ground and before any window's page, which is the painter's
    // order the game itself used (SQ-0706): the plate is something the
    // game DREW, and a page is a colour it was told to present on.
    v6::blit_story_gfx(&mut canvas, layout.story_gfx);
    // SQ-0704: a chrome window that named its own page paints it
    // into its unpainted pixels here (ZMSD §8.8.3.2), so the ring
    // bands ship self-contained instead of leaving the icons'
    // clear ground for the terminal to colour in (Zork Zero's
    // room icons came out on an opaque black box). Same live
    // `honor_game_colours` gate as the pane flood above.
    // The painted ground goes under the ring's art and glyphs
    // and before the pages claim the rest (SQ-0706).
    v6::blit_paint_ground(
        &mut canvas,
        state.v6_paint.borrow().as_deref(),
        v6::TextLayer::SkipGlyphRows(&glyph_rows),
        state.v6_text.cell(),
    );
    // SQ-0883: the INK layer, frozen — art, glyph ink and painted
    // ground, before any window's PAGE floods the rest. The border
    // probe below reads THIS: a page is a colour a window was told
    // to present on, not something the game drew, and a probe that
    // cannot tell the two apart measures a flank's whole width as
    // its border rule. `build_chrome_canvas` freezes its own art
    // layer one step earlier for the same reason (SQ-0727/0500):
    // opaque is not painted.
    let ink = canvas.clone();
    if state.config.honor_game_colours {
        v6::fill_window_pages(
            &mut canvas,
            &ring_chrome,
            layout.story,
            &state.colors,
            v6::TextLayer::SkipGlyphRows(&glyph_rows),
            state.v6_text.cell(),
        );
        // …and the story window's own page under the pixels the
        // ring bands ship (SQ-0704, hybrid half). Raster flattens
        // its whole canvas opaque before shipping; hybrid ships
        // only these bands, and they overlap the story box — the
        // sliver under a top banner, and the flanks. A pixel left
        // transparent there is the TERMINAL's to resolve, which is
        // why the icons kept coming out on the terminal background
        // after the chrome half of this fix landed.
        v6::fill_story_page_clear(&mut canvas, layout.story, &state.colors);
    } else {
        // SQ-0716: colours declined, but a window the game has
        // PAINTED INTO still gets its page — that page is the
        // ground its own drawing sits on, not a palette
        // preference. See `fill_painted_window_pages`.
        v6::fill_painted_window_pages(
            &mut canvas,
            &layout.chrome,
            layout.story,
            &state.colors,
            state.v6_paint.borrow().as_deref(),
            state.v6_text.cell(),
        );
    }
    // SQ-0903: the carve that used to be here is GONE, and the
    // proof it is safe is that it had already stopped removing
    // anything: `build_chrome_canvas` is now told which rows the
    // ring claimed (`glyph_rows`, above) and never paints them, so
    // there is nothing left on them to erase. Measured across the
    // corpus with the carve still in place — zork0, arthur, shogun,
    // journey, advent and mysterious01 all reported **0** pixels
    // removed, which is the oracle SQ-0903 asked for.
    //
    // What it used to do is worth keeping in view, because the rule
    // survives even though the code does not: on a row the ring
    // draws with GLYPHS, this canvas keeps artwork and nothing else
    // (SQ-0750). It is enforced a step earlier now, by not painting,
    // rather than a step later by erasing.
    // SQ-0511 fix: in the Menu plan the side flanks are drawn at the
    // UNIFORM scale (aspect preserved — Journey's left picture column
    // is NOT vertically stretched); only each flank's full-height
    // divider/border column is extended down through the reclaimed gap
    // to the bottom-anchored menu. Compute those narrow extension bands
    // up front so their cache keys join the live set (else they'd be
    // pruned and re-encoded every frame). The Frame plan (Zork0/Shogun)
    // still stretches its whole flank (border art, no story picture).
    // SQ-0758: BOTH of a flank's border columns, per flank strip —
    // `(strip, inner rule, outer border)`. One probe, run from each
    // side, so the panel's extent and the two borders that bound it
    // come out of the same calculation instead of the band's rect.
    // SQ-0779: and under every OTHER plan too — but only for a border
    // the game printed as a CHARACTER. The extension used to be a
    // Menu-plan privilege, so a pane short enough to leave no letterbox
    // slack (`slack == 0` → `Letterbox`, i.e. any pane whose rows are
    // at or below the scaled native height) got no border columns at
    // all: Journey's Amiga frame lost BOTH of its left flank's rules
    // into the picture band that spans them, and its right-hand flank —
    // one rule wide, with no art in it — classified art-less and was
    // skipped, so the frame simply had no sides between its `┌─┐` and
    // its `└─┘`. Reported at a 121x36 terminal, correct at 117x64;
    // the discriminator is the pane's ASPECT, not its width.
    //
    // Reserved to the GLYPH ink outside the Menu plan, which is what
    // keeps the corpus still: an artwork flank (Zork Zero's, Shogun's,
    // Arthur's side columns) comes back `Band` here and is dropped, so
    // those frames draw exactly the bands they drew before. The
    // extension's bottom is the flank strip's own, since outside the
    // Menu plan there is no bottom-anchored strip to reach down to.
    //
    // SQ-0830 took Journey back out of this arm: it holds the Menu plan
    // at every aspect now, so the 121x36 pane above is a menu plan and
    // its rules come from where they always came from at a tall one. The
    // generalisation stands on its own merits — a glyph border under any
    // other plan is still a character the game printed, and still must
    // not be shipped as a bitmap of itself.
    let glyph_borders_only = !matches!(plan, BottomPlan::Menu);
    let flank_borders: Vec<(Rect, Option<FlankBorderExt>, Option<FlankBorderExt>)> = strips
        .iter()
        .filter_map(|s| match s {
            ChromeStrip::Art(role, r) if role.is_flank() => {
                let bottom = if glyph_borders_only { r.bottom() } else { viewport.bottom() };
                let ext = |which| {
                    flank_border_extension(
                        *r, area, viewport, &scale, cell_px, story, native, &ink, &gfx,
                        &chrome_runs, bottom, which,
                        state.v6_text.cell(),
                    )
                    .filter(|(_, ink)| !glyph_borders_only || matches!(ink, BorderInk::Glyph { .. }))
                };
                let inner = ext(FlankBorder::Inner);
                // A flank only one border wide — Journey's
                // right-hand column is exactly that — finds
                // the SAME run from both sides. Drawing it
                // twice would put two bands on one cache key.
                let outer = ext(FlankBorder::Outer)
                    .filter(|(o, _)| inner.is_none_or(|(i, _)| i != *o));
                Some((*r, inner, outer))
            }
            _ => None,
        })
        .collect();
    // SQ-0779, second pass: and it must not overlap it in the SOURCE
    // either. A band's crop is its destination rect mapped back through
    // the letterbox scale, so trimming the destination by whole terminal
    // columns lands the crop's edge inside the border's own 8-pixel text
    // cell — the game's rule was still in the picture, moved one column
    // in rather than removed. `clear_text_rows` has always carved a text
    // strip's native ROWS out of this canvas for exactly this reason; a
    // stamped border COLUMN needs the same carve, and gets it here. Only
    // over the story's own native rows, which is the span the glyph
    // path's content test proved carries no artwork.
    {
        let cols: Vec<(u32, u32)> = flank_borders
            .iter()
            .flat_map(|(_, i, o)| i.iter().chain(o.iter()))
            .filter_map(|(_, ink)| match ink {
                BorderInk::Glyph { native, .. } => Some(*native),
                BorderInk::Band(_) => None,
            })
            .collect();
        let rows = (story.y_px as u32, story.y_px as u32 + story.h_px as u32);
        v6::clear_text_columns(&mut canvas, &cols, rows);
    }
    // SQ-0779, the user's ruling: **if a game draws a border, the
    // artwork should not overlap it.** A flank strip runs the whole
    // width of the flank, borders included, and outside the Menu plan
    // that whole strip is one uploaded band — so the picture's own
    // placement rect covered the columns the frame's rules stand in.
    // The rules were then unstampable (a glyph must not be written
    // over an image that composites above the cells) and reached the
    // screen, if at all, as a resampled bitmap of themselves, which is
    // the thing hybrid exists not to do.
    //
    // So the LAYOUT stops short of them: the art's allocated span is
    // trimmed to end where the border column begins, and the border is
    // stamped as the character the game printed. No pixel is lost —
    // the glyph path's content test has already established that the
    // graphics-only canvas is clear across those native columns — and
    // the Menu plan needs none of it, since its panel fill and its art
    // rect are already bounded by the two rules (SQ-0747/0758).
    if glyph_borders_only {
        for s in &mut strips {
            let ChromeStrip::Art(role, r) = s else { continue };
            if !role.is_flank() {
                continue;
            }
            let Some((_, inner, outer)) = flank_borders.iter().find(|(sr, _, _)| sr == r) else {
                continue;
            };
            // Which rule stands on which side: a LEFT flank is bounded
            // by its outer border on the left and the story-side rule
            // on the right; a right flank the other way about.
            let (lo, hi) = if r.x < viewport.x { (outer, inner) } else { (inner, outer) };
            let (mut x0, mut x1) = (r.x, r.right());
            if let Some((e, _)) = lo.filter(|(e, _)| (x0..x1).contains(&e.x)) {
                x0 = e.right();
            }
            if let Some((e, _)) = hi.filter(|(e, _)| (x0..x1).contains(&e.x)) {
                x1 = e.x;
            }
            *r = Rect::new(x0, r.y, x1.saturating_sub(x0), r.height);
        }
        strips.retain(|s| !matches!(s, ChromeStrip::Art(_, r) if r.width == 0));
    }
    let strips = strips;
    let divider_exts: Vec<FlankBorderExt> = flank_borders
        .iter()
        .flat_map(|(_, i, o)| i.iter().chain(o.iter()).copied())
        .collect();
    // SQ-0747: and the rects the flank PANELS are drawn at, for the
    // same reason. A Menu-plan flank's art goes to `menu_flank_panel`'s
    // DEST rect, not to the strip's own rect, and the band cache is
    // keyed on the rect a band is drawn at — so the strip rect alone in
    // the live set left the panel's key unclaimed. `retain_chrome_bands`
    // evicted it every frame, and one eviction clears the WHOLE cache,
    // so every band re-encoded and re-uploaded on every frame (three
    // uploads a frame on Journey's menu, for pixels the terminal already
    // had — the user's `band uploads since launch: 78` over 26 ring
    // frames). Resolved once, up here, and reused by the draw below.
    let flank_panels: Vec<(Rect, FlankPanel)> = if matches!(plan, BottomPlan::Menu) {
        strips
            .iter()
            .filter_map(|s| match s {
                ChromeStrip::Art(role, r) if role.is_flank() && strip_has_art(r) => {
                    // BOTH of the flank's borders: the inner rule is
                    // what the art insets away from, and the two
                    // together are what the panel fill stops short of
                    // (SQ-0747). The old lookup took the first
                    // extension inside the strip, which is the OUTER
                    // border now that one is produced.
                    let bord = flank_borders.iter().find(|(sr, _, _)| sr == r);
                    let inner = bord.and_then(|(_, i, _)| i.map(|(e, _)| e));
                    let outer = bord.and_then(|(_, _, o)| o.map(|(e, _)| e));
                    menu_flank_panel(*r, viewport, &scale, cell_px, story, native, &gfx, inner, outer)
                        // SQ-0946: and the panel's ground stops at the
                        // game SCREEN's edge. `menu_flank_panel` bounds
                        // the fill by the flank's two BORDER columns and
                        // falls back to the band when a border is not
                        // found — which is Journey's IBM PC press, whose
                        // outer rule is a reverse-video space the border
                        // probe does not return. The band runs to the
                        // pane edge, so the fill ran into the letterbox
                        // margin: nine columns of panel colour down the
                        // left of a 98x37 pane with `v6_pixel_lock` on,
                        // against a bare right margin. That asymmetry IS
                        // the report; the art itself was centred to the
                        // cell at every width measured.
                        .map(|(bg, fill, dest, crop)| {
                            let (lo, hi) = v6::screen_cols(&scale, native.0, cell_px, area);
                            let x = fill.x.max(lo);
                            let w = fill.right().min(hi).saturating_sub(x);
                            (*r, (bg, Rect::new(x, fill.y, w, fill.height), dest, crop))
                        })
                }
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    // SQ-0698/SQ-0781: the side flanks of Arthur, Shogun and Zork
    // Zero, TILED down to the band instead of stretched to it. The
    // source is composed in native pixels at the uniform scale, so
    // the flank keeps the top plate's horizontal factor and gains no
    // vertical one — the whole point, since the stretch it replaces
    // ran to 2.2x (Zork Zero) and 3.0x (Shogun) of the horizontal at
    // a 117x64 terminal. The Menu plan is excluded: Journey's frame
    // is glyphs, not artwork (SQ-0750), and its flank is a picture
    // column centred in a panel rather than a border to extend.
    let tiled_flanks: Vec<(Rect, TiledFlank)> = if matches!(plan, BottomPlan::Menu) {
        Vec::new()
    } else {
        strips
            .iter()
            .filter_map(|s| match s {
                ChromeStrip::Art(role, r) if role.is_flank() => {
                    flank_tiled_source(*r, area, &scale, cell_px, native, &canvas, &gfx)
                        .map(|img| (*r, img))
                }
                _ => None,
            })
            .collect()
    };
    // SQ-0818: how finely a FULL-WIDTH art strip's upload is cut.
    //
    // Granularity is backend-conditional, and only this side of the
    // renderer knows the picker. Kitty and iterm2 tile: the extra cost
    // is one control block and a rounded-up last chunk per tile, and
    // the payload is byte for byte the same pixels. SIXEL DOES NOT —
    // every sixel image carries its own palette definition, so N tiles
    // would mean N palettes where the strip had one, a real first-frame
    // regression for no gain. Halfblocks does not care either way:
    // ratatui's own cell diff already sends it only the dirty cells.
    let tile_cols = match picker.protocol_type() {
        ratatui_image::picker::ProtocolType::Kitty
        | ratatui_image::picker::ProtocolType::Iterm2 => BAND_TILE_COLS,
        ratatui_image::picker::ProtocolType::Sixel
        | ratatui_image::picker::ProtocolType::Halfblocks => 0,
    };
    // …and only a strip that is NOT a flank, which is asked by role
    // rather than inferred from the draw: `flank_panels` and
    // `tiled_flanks` each compose their own source image from geometry
    // that is not a straight sub-rect of the scaled canvas, and a flank
    // the plain crop draws (SQ-0898 removed the stretch arm, so a piece
    // needing no extension lands here) is tall and thin — column tiles
    // would buy it nothing.
    let art_tiles = |role: v6::BandRole, r: Rect| -> Vec<Rect> {
        if role.is_flank() {
            vec![r]
        } else {
            band_tiles(r, tile_cols)
        }
    };

    // SQ-1187: everything above is the frame's COMPUTE half — pure
    // derivation from the window model, the pane, the config and the
    // theme. It is entered only when `v6_hybrid_gen` says an input
    // moved; the draw half replays this value otherwise.
    // An Art strip with no art behind it is skipped below and never
    // drawn, so its key must NOT be claimed here: a live key nothing
    // re-places keeps a cached upload the terminal is no longer being
    // pointed at, which is the stale-placement shape SQ-0587 records
    // (SQ-0747). Only `strips` is filtered — the menu strips below
    // draw unconditionally.
    // SQ-0818: a tiled strip claims its TILES' keys, never the whole
    // strip's — a live key nothing re-places is a stale placement
    // (SQ-0587), and a key the draw claims that the live set does
    // not is evicted every frame, which clears the WHOLE cache.
    let live: std::collections::HashSet<_> = strips
        .iter()
        .filter(|s| !matches!(s, ChromeStrip::Art(_, r) if !strip_has_art(r)))
        .filter_map(|s| match s {
            ChromeStrip::Art(role, r) => Some((*role, *r)),
            ChromeStrip::Text(..) => None,
        })
        .flat_map(|(role, r)| art_tiles(role, r))
        // The menu's own strips are drawn whole, below.
        .chain(menu_strips.iter().filter_map(|s| match s {
            ChromeStrip::Art(_, r) => Some(*r),
            ChromeStrip::Text(..) => None,
        }))
        .map(|r| (crate::render::graphics::BandSlot::Art as u8, r.x, r.y, r.width, r.height))
        // A GLYPH border (SQ-0750) uploads nothing, so it claims no
        // cache key: a live key nothing re-places is the stale
        // placement SQ-0587 records.
        .chain(divider_exts.iter().filter(|(_, ink)| matches!(ink, BorderInk::Band(_))).map(|(r, _)| {
            (crate::render::graphics::BandSlot::DividerExtension as u8, r.x, r.y, r.width, r.height)
        }))
        .chain(flank_panels.iter().map(|(_, (_, _, d, _))| {
            (crate::render::graphics::BandSlot::Art as u8, d.x, d.y, d.width, d.height)
        }))
        .collect();
    // Record the letterbox geometry for click→game-pixel
    // mapping (Lane M): the chrome ring shares this scale. In
    // the Menu plan the interactive read_char region IS the
    // bottom-anchored command strip, so record ITS scale — a
    // single V6ClickMap is one linear transform, and the menu
    // is where clicks are meaningful (story-region clicks map
    // through the menu offset, but the game reads only the
    // menu pixels). Extend/Letterbox use one scale everywhere.
    //
    // Asked of the PLAN and not of `menu.is_some()` since SQ-1008,
    // which gave the Extend/Frame plans a bottom-anchored band of
    // their own for content the game keeps below the story window
    // (Arthur's boxed `hint` message). That band is paint, not a
    // picker: the interactive region on those frames is still the
    // story, so they must keep inverting through the story scale.
    // Byte-identical for all four plans as they stood — `menu` was
    // `Some` under exactly the `Menu` plan and nowhere else.
    // SQ-0550: that scale alone inverts the menu WRONG. The menu
    // is a TEXT strip, and `draw_chrome_text_strip` packs its game
    // rows onto CONSECUTIVE terminal rows from the strip's top
    // (SQ-0543) rather than placing them through the scale — so the
    // linear inverse drifts by the difference between the two row
    // pitches, and Journey's player had to click one line below the
    // command they wanted (two by the bottom row). Hand the map the
    // strip's row mapping so clicks inside it invert by row index.
    // The count is the strip's GAME rows, which can be fewer than
    // its classified height: the classifier places runs through the
    // scale (leaving gaps the bridge rule absorbs) while the draw
    // packs them tight, so anything past the last packed row falls
    // through to the letterbox.
    //
    // SQ-0747: and the menu is not always a `menu_strips` strip. Only
    // the reclaim plans anchor one to the pane's bottom; under the
    // `Letterbox` plan — any pane with no vertical slack, which is
    // where this quest's report comes from — the very same command
    // menu is an ordinary TEXT strip of the RING, drawn by the very
    // same packing function, and `menu_strips` is empty. The map then
    // got `None` and inverted the whole pane linearly, which is
    // SQ-0550's defect one plan over: measured off `Journey - The
    // Quest Begins.adf` (release 30 / serial 890322), clicking `Cast`
    // exactly where it is drawn is ACCEPTED at 119x34 (menu plan) and
    // MISSED at 115x31, 150x41 and 234x65 (letterbox). So fall back to
    // the ring's own bottom-most text strip below the story viewport,
    // which is the same strip by another name.
    //
    // SQ-0830 removed the case that motivated this: a game with a menu
    // strip now takes the `Menu` plan at any pane aspect, so those three
    // sizes are menu plans today and `menu_strips` carries the band. The
    // fallback stays because it is not about Journey — any plan can leave
    // a text strip below the viewport, and packing is how every one of
    // them is drawn.
    //
    // The third element is the native PIXEL top of the row drawn
    // at the strip's first terminal row — the run's own `y`, not
    // its row index times 16 (SQ-0951). They agree wherever the
    // game prints on the 16px grid, which Journey does; they do
    // not where it prints off it, and the index then names a slot
    // the text is not in.
    let packed = |r: &Rect, runs: &[crate::engine::PxText]| {
        let rows = runs.iter().map(|t| state.v6_text.cell().row_of(t.y));
        let first = rows.clone().min()?;
        let last = rows.max()?;
        let first_top = runs.iter().map(|t| t.y.max(1) - 1).min()?;
        Some((r.y, r.height.min(last - first + 1), first_top))
    };
    let mut packed_text: Vec<crate::render::graphics::PackedText> = Vec::new();
    if let Some(rows) = menu_strips
        .iter()
        .find_map(|s| match s {
            ChromeStrip::Text(r, runs) => packed(r, runs),
            ChromeStrip::Art(..) => None,
        })
        .or_else(|| {
            strips.iter().rev().find_map(|s| match s {
                ChromeStrip::Text(r, runs) if r.y >= viewport.bottom() => packed(r, runs),
                _ => None,
            })
        })
    {
        // A chrome text strip packs its ROWS and still places
        // its columns through the scale, so it publishes no
        // column mapping and x keeps the proportional inverse.
        packed_text.push(crate::render::graphics::PackedText { rows, cols: None });
    }
    // SQ-0938: and the STORY BOX, which packs BOTH axes.
    //
    // The in-box run packing draws one terminal cell per native
    // text cell — rows from the box's top since SQ-0892, columns
    // from its left since SQ-0937 — so a click inside it has to
    // invert the same way or it lands on a different line than
    // the one under the pointer. Zork Zero's and Shogun's hint
    // menus say "(Or use mouse.)", so this is the screen working,
    // not a nicety.
    //
    // Recorded from the SAME numbers the drawing used
    // (`viewport`, `vp_native`), because a click map derived
    // independently of the draw is a second implementation of the
    // same geometry and will drift from it.
    //
    // SQ-0951: which is exactly what the COLUMN span had become.
    // A promoted story GRID — Zork Zero's and Shogun's InvisiClues
    // topic list — was not drawn by the in-box run packing at all:
    // `render_node` handed it to `draw_grid`, which placed the
    // GAME's screen (`cols` wide) CENTRED in the pane rather than
    // flush to the viewport's left edge. At a 190x60 pane that is a
    // 58-column grid in a 138-column viewport, so every topic was
    // drawn forty columns right of where this map claimed it was,
    // and the player had to click far to the LEFT of a topic to
    // select it. SQ-0951 taught the map to follow that centring.
    //
    // SQ-1074 removed the centring — a v6 window has an absolute
    // native origin and is drawn where the ring put it — so the two
    // arms collapse back into one and the map is once again the
    // viewport's own first column, for prose and grid alike. That
    // is what SQ-0951 wanted in the first place: "recorded from the
    // SAME numbers the drawing used", rather than a second
    // implementation that has to chase the first.
    if viewport.width > 0 && viewport.height > 0 {
        let (left, cols) = (viewport.x, viewport.width);
        packed_text.push(crate::render::graphics::PackedText {
            rows: (viewport.y, viewport.height, vp_native.1 as u16),
            cols: Some((left, cols, vp_native.0 as u16)),
        });
    }
    // Which Art strips have real art behind them, asked of the oracle
    // ONCE here so the draw half (which replays without the canvases'
    // borrows) can skip artless strips by lookup (SQ-1187).
    let art_backed: std::collections::HashSet<(u16, u16, u16, u16)> = strips
        .iter()
        .filter_map(|s| match s {
            ChromeStrip::Art(_, r) if strip_has_art(r) => Some((r.x, r.y, r.width, r.height)),
            _ => None,
        })
        .collect();
    HybridFrame {
        key: hkey,
        canvas,
        gfx,
        scale,
        menu,
        plan_is_menu: matches!(plan, BottomPlan::Menu),
        ring_plan,
        ring_clip,
        image_scale,
        lock_inapplicable,
        lock_fallback,
        viewport,
        vp_native,
        strips,
        menu_strips,
        art_backed,
        flank_borders,
        divider_exts,
        flank_panels,
        tiled_flanks,
        tile_cols,
        live,
        packed_text,
        over_art_runs,
    }
}

/// A cheap change key for the whole v6 raster composite (SQ-0469). It folds
/// EVERY input the raster branch reads to build the native canvas — the v6 window
/// model, the transcript, the live input line, scroll/pager/caret state, the pane
/// size + font, and the themed colours — into one `u64`. When the key is
/// unchanged the entire rebuild + resize + encode is skipped, so idle and
/// keystroke frames cost only this hash (microseconds) instead of milliseconds.
///
/// A missed input here is a stale-frame bug, so the coverage is deliberately
/// generous (it hashes the built model's render fields rather than trusting a
/// hand-maintained zvm mutation counter — the model is observed, so no v6 paint
/// or erase can slip past). The inputs are audited in the SQ-0469 report.
///
/// "The model is observed" stopped covering everything the moment the painted
/// ground arrived (SQ-0706): that surface rides BESIDE the window tree, not
/// inside it, so a game whose drawing lands there alone moved no input this key
/// was reading. It is folded in below.
pub fn v6_raster_gen(items: &[PositionedWindow], state: &AppState, area: Rect, picker: &ratatui_image::picker::Picker) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Pane geometry + font size (drive the story cols/rows and the encode target).
    (area.width, area.height).hash(&mut h);
    let fs = picker.font_size();
    (fs.width, fs.height).hash(&mut h);
    // The v6 window model: each window's box geometry plus its render content —
    // graphics by version stamp (not pixels), text by its positioned runs and
    // colours. This observes the composited output, so any paint/erase/scroll or
    // colour change on the zvm side is captured without a bespoke counter.
    for pw in items {
        (pw.x, pw.y, pw.w, pw.h, pw.x_px, pw.y_px, pw.w_px, pw.h_px, pw.left_margin, pw.right_margin).hash(&mut h);
        match &pw.node {
            WinNode::Graphics(g) => {
                g.win.hash(&mut h);
                g.version.hash(&mut h);
            }
            WinNode::Grid(g) => {
                (g.bg, g.fg, g.cursor, g.cursor_active).hash(&mut h);
                for t in &g.px_texts {
                    (t.x, t.y, t.style, t.fg, t.bg).hash(&mut h);
                    t.text.hash(&mut h);
                }
            }
            WinNode::Buffer(b) => {
                (b.bg, b.fg, b.primary).hash(&mut h);
                // A SECONDARY prose window's lines are drawn into the composite
                // (SQ-0729), so a change to them must rebuild it — without this the
                // cached canvas outlives the text it was built from.
                if !b.primary {
                    b.lines.hash(&mut h);
                }
            }
            _ => {}
        }
    }
    // The painted ground (SQ-0706), which `build_v6_raster_canvas` blits under
    // everything above and which the window model does not describe. scopa draws
    // its entire card table with `erase_window` fills and publishes no Graphics
    // window at all, so moving the selection outline from one hand card to the
    // next repaints 1120 pixels of ground and changes NOTHING in the model: the
    // key held still, `v6_wants_build` said no, and the already-uploaded frame
    // stayed on screen while the game went on to play the card the player had
    // actually picked. The first selection looked right only because it also
    // relabelled the confirm button "Choose" -> "OK", and a later one looked
    // right whenever it happened to add or drop a board highlight — both are
    // model changes, so both bumped the key and dragged the outline along with
    // them (SQ-0788).
    //
    // Hashed by CONTENT, like the hybrid path's per-band freshness hash: a
    // mutation counter on the zvm side is the "hand-maintained" thing this key
    // was written to avoid, and it is what would rot silently here. Measured in
    // release over scopa's 640x400 ground: 0.30 ms against the 0.77 ms canvas
    // rebuild this gate exists to skip, and exactly 0 for the v6 games that
    // paint nothing (`v6_paint` is `None`, so there is no buffer to walk).
    match state.v6_paint.borrow().as_deref() {
        Some(ground) => {
            1u8.hash(&mut h);
            ground.dimensions().hash(&mut h);
            ground.as_raw().hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    // App-side inputs to build_main_text + the pager/caret.
    state.transcript_gen.hash(&mut h);
    state.transcript_images.len().hash(&mut h);
    state.input.value.hash(&mut h);
    state.effective_transcript_scroll().hash(&mut h);
    matches!(state.focus, crate::state::Focus::Game).hash(&mut h);
    state.pager.active.hash(&mut h);
    // The themed colours the raster resolves (default fg/bg + the [more] prompt);
    // a theme switch changes these even when the model is byte-identical.
    // A live `/set-game-colours` toggle changes the composite's page/ink
    // resolution without touching the model — it must invalidate the canvas.
    state.config.honor_game_colours.hash(&mut h);
    // SQ-1032: and the MODE, which now decides the canvas's own height. `/set-v6-render`
    // between raster and extended moves no window, no run and no pixel of the model, so
    // without this the cached composite outlives the mode it was built for.
    state.config.v6_render.hash(&mut h);
    let tbg = state.colors.theme.get("transcript").style;
    style_fg_rgba(tbg, image::Rgba([220, 220, 220, 255])).0.hash(&mut h);
    style_bg_rgba(tbg, image::Rgba([0, 0, 0, 255])).0.hash(&mut h);
    let mp = state.colors.theme.get("more_prompt").style;
    style_fg_rgba(mp, image::Rgba([220, 220, 220, 255])).0.hash(&mut h);
    style_bg_rgba(mp, image::Rgba([0, 0, 0, 255])).0.hash(&mut h);
    // The lit reveal (SQ-1138), which `draw_story_text` composites INTO the canvas
    // — so without it here the gate skips the rebuild and the reveal lights
    // nothing at all. That is not hypothetical: arming a reveal moves no window,
    // no run and no pixel of the model, changes no transcript line and no input
    // character, so every other input to this key holds perfectly still. The whole
    // feature would be dark and every reason for it correct.
    //
    // Hashed by CONTENT, like the painted ground above: the words, and the ink
    // they light in. `is_lit` is wall-clock, so the key also changes on its own
    // when a reveal expires — which is what repaints the prose back to the story's
    // own colour without anyone having to remember to.
    match state.reveal.as_ref().filter(|r| r.is_lit()) {
        Some(r) => {
            1u8.hash(&mut h);
            r.words.hash(&mut h);
            state.colors.theme.get("transcript_reveal").style.fg.hash(&mut h);
        }
        None => 0u8.hash(&mut h),
    }
    h.finish()
}

/// Build the main-window text block for the pixel composite: the newest visible
/// wrapped transcript lines that fit the primary window's rows, plus the live
/// input line and caret column.
/// Build the v6 raster story text: wrap the transcript to the window width,
/// then place window-0 inline pictures (the `transcript_images` sidecar)
/// according to their `ImageAlign` (SQ-0470 follow-up). A `MarginLeft` image
/// floats — it occupies no text row, anchors at the next wrapped row, and
/// indents the `pic_height/8` rows beside it (Zork Zero's drop-cap idiom; the
/// indent comes from the game's own `set_margins` when it was captured). Every
/// other alignment (InlineUp/Down/Center, MarginRight — e.g. Shogun's ship
/// splash) is a full-width band: it reserves `pic_height/8` blank text rows so
/// prose stops above it and resumes below, never beside or over it. Keeps the
/// newest `rows-1` wrapped rows (one row is left for the input line).
pub fn build_main_text(state: &AppState, cols: u16, rows: u16) -> (crate::render::v6_layout::MainText, RasterMetrics) {
    // THE WRAP IS NOT DONE HERE ANY MORE (SQ-1034). It lives in
    // `render::wrap_cache`, alongside the cell path's, under one key type and one
    // append-or-rebuild rule — because two copies of "has the wrap moved?" is
    // exactly what drifted: this path had no cache at all and sat behind a
    // whole-canvas gate that hashes `input.value`, so one keystroke re-wrapped
    // 20,000 turns of scrollback (25.058 ms measured).
    //
    // What is left here is the WINDOWING, which is per frame and cheap: the
    // newest visible rows, the floats that reach into them, the live input line
    // and the caret. Raster's `cols` come from `story_prose_box` over the NATIVE
    // v6 screen rect, so they do not move with the pane — which is why this path
    // takes the append branch essentially always.
    crate::render::wrap_cache::raster_wrap_refresh(state, cols);
    let cache = state.raster_wrap.borrow();
    let cache = cache.as_ref().expect("refreshed above");
    let wrapped = &cache.rows;
    let wrapped_styles = &cache.styles;
    let line_starts = &cache.starts;
    // One row is reserved for the live input line, so the transcript body budget
    // is `rows - 1` — this is the raster viewport height the [more] pager and the
    // scroll keybindings measure against.
    let budget = rows.saturating_sub(1) as usize;
    let total = wrapped.len();
    let max_scroll = total.saturating_sub(budget);
    // Rows-from-bottom scroll offset (0 = newest at the bottom), clamped so it
    // never scrolls past the oldest row. Same scroll model as the terminal
    // transcript (`effective_transcript_scroll`), so the shared scroll keys and
    // the [more] pager (SQ-0404) drive the raster and terminal paths identically:
    // when the user scrolls back the visible slice shifts up in lockstep. (SQ-0455)
    let scroll = (state.effective_transcript_scroll() as usize).min(max_scroll);
    let mut end = total.saturating_sub(scroll);
    let mut start = end.saturating_sub(budget);
    // Top-anchor the post-clear screen, exactly as the cell path does
    // (`window_wrapped_rows`, SQ-0305/0640): at the bottom of the scrollback, a
    // game screen-clear pins its output to the TOP of the box with blanks below,
    // instead of bottom-sticking and dragging pre-clear history back into view.
    // Shogun's title needs it (SQ-0728): the SQ-0697 freeze retires nine banner
    // lines as paint and marks the clear, and window 0's new box is four rows —
    // bottom-sticking redrew the tail of the banner it had just frozen up top,
    // across the menu, instead of the one line the game printed into the new box.
    // Only while the post-clear content still fits; once it overflows, the box
    // scrolls normally.
    // Shared with the cell path so an anchor at the very end of the transcript —
    // cleared, nothing printed since — reads as an EMPTY screen on both, rather
    // than as an absent anchor that bottom-sticks the erased scrollback (SQ-0748).
    let anchor_row = (scroll == 0)
        .then(|| crate::render::transcript::anchor_row_at(line_starts, total, state.clear_anchor))
        .flatten();
    if let Some(a) = anchor_row.filter(|&a| total - a <= budget) {
        start = a;
        end = total;
    }
    let visible_len = end - start;
    let lines = wrapped[start..end].to_vec();
    // Emphasis travels with the visible slice. `wrapped_styles` self-pads — it
    // only reaches the last row that carried any emphasis — so a row past its end
    // is all-roman rather than missing.
    let styles: Vec<Vec<u8>> =
        (start..end).map(|r| wrapped_styles.get(r).cloned().unwrap_or_default()).collect();
    // Shift floats into the visible window; keep those still (partially) visible.
    let floats: Vec<crate::render::v6_layout::RasterFloat> = cache
        .floats
        .iter()
        .filter_map(|f| {
            let rel = f.row as i64 - start as i64;
            (rel + f.rows as i64 > 0 && rel < visible_len as i64).then(|| crate::render::v6_layout::RasterFloat {
                row: rel as i32,
                rows: f.rows,
                reserve_cols: f.reserve,
                text_col: f.text_col,
                img_col: f.img_col,
                img: std::sync::Arc::clone(&f.img),
            })
        })
        .collect();
    let input = state.input.value.clone();
    let cursor_col = input.chars().count().min(cols.saturating_sub(1) as usize) as u16;
    // Show the input line + caret whenever the view is at the bottom — scrolled-back
    // history must not be overwritten by the live line (matching the terminal
    // transcript's `effective_scroll == 0` guard).
    //
    // Deliberately NOT gated on host focus. It used to be, which meant the caret and
    // everything you had typed vanished the moment the keyboard went to the map —
    // opening a room panel, or reaching the inspector via select-room, hid your own
    // half-typed command with no indication it was still buffered. The Z-machine
    // transcript path has never had such a gate, so the two engines disagreed too.
    // Whether keystrokes currently reach the story is the focus HIGHLIGHT's job, not
    // the input line's.
    let awaiting = scroll == 0;
    let main = crate::render::v6_layout::MainText { lines, styles, input, cursor_col, awaiting, floats };
    let metrics = RasterMetrics {
        total_rows: total.min(u16::MAX as usize) as u16,
        viewport_rows: budget.min(u16::MAX as usize) as u16,
        max_scroll: max_scroll.min(u16::MAX as usize) as u16,
        first_visible_row: start.min(u16::MAX as usize) as u16,
    };
    (main, metrics)
}

/// Scroll/pager geometry the raster story text reports back so the [more] pager
/// (SQ-0404) and the transcript scroll keybindings engage on the raster path
/// exactly as they do on the terminal transcript. Rows are counted in the
/// raster's own 8-px text lines. (SQ-0455)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterMetrics {
    /// Total wrapped transcript rows this frame (the pager needs the true total).
    pub total_rows: u16,
    /// The transcript body viewport height in rows (`story_box_rows - 1`, the
    /// input line reserved).
    pub viewport_rows: u16,
    /// The largest meaningful scroll offset (`total_rows - viewport_rows`).
    pub max_scroll: u16,
    /// Absolute wrapped-row index drawn at the top of the visible slice (for the
    /// published `TranscriptGeom`).
    pub first_visible_row: u16,
}

/// How deep a chrome run must sit before the HYBRID path will treat a screen as a
/// painted MENU takeover (see the `has_menu` gate in the `Layered` arm). A run
/// this shallow is ordinary top-of-screen status chrome even when it happens to
/// land inside a story box that starts at row 0. (SQ-0478/SQ-0494)
const STATUS_BAND_ROWS: u16 = 4;

/// Render a v6 PAINTED text screen (menus, hints — SQ-0477/0478) as absolutely-
/// positioned terminal text. Each run is quantized to its native cell
/// (`col = (x-1)/8`, `row = (y-1)/16` — the non-square 8×16 v6 cell) and stamped at that pane-relative cell,
/// honoring reverse video — Shogun's boot-menu selection is a reverse-video run,
/// so this is what makes the selection caret visible. Menus are absolutely
/// positioned (NOT left/center/right anchor groups like the status band).
///
/// Only native rows inside `rows` are drawn, each placed at `area.y + row +
/// shift`. The cell path calls this twice (SQ-0491): once for the runs
/// INSIDE the story box (`shift = 0` — Shogun's boot menu keeps its native rows
/// over the transcript) and once for the command band BELOW it (a negative or
/// positive `shift` that packs those rows against the pane's bottom edge, so
/// Journey's menu stays locked to the bottom at any pane height). A story-less
/// menu screen passes the whole range with no shift to stamp the entire pane.
/// Shared by the cell and hybrid (no story window) paths so both present a
/// painted screen identically.
/// Resolve a v6 painted run's packed fg/bg (see [`crate::engine::PxText`]) plus
/// its reverse bit onto a `base` theme [`Style`], for the CELL render paths (the
/// cell-path status band, the painted-screen overlay, and the hybrid story-strip
/// overlay). Mirrors the v1-5 / Glulx cell rule (`cell_style`): a run whose
/// channel carries an EXPLICIT game colour (see [`v6_layout::packed_explicit`])
/// replaces that channel; a Default or Standard-0/1 sentinel is inheritance, so
/// the theme keeps the channel. Gated on the ink's `honor` exactly like every
/// other engine's colour path — colours OFF ⇒ the theme `base` is returned
/// untouched. The reverse bit toggles REVERSED (the terminal performs the fg/bg
/// swap), so an explicit pair under reverse shows swapped and Shogun's
/// Default/Default, non-reversed runs collapse to exactly `base`. (SQ-0488)
fn v6_run_style(
    base: ratatui::style::Style,
    fg: u32,
    bg: u32,
    style_bits: u8,
    ink: TextInk,
) -> ratatui::style::Style {
    let mut s = base;
    if ink.honor() {
        if crate::render::v6_layout::packed_explicit(fg) {
            s = s.fg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(fg), ink.colors()));
        }
        if crate::render::v6_layout::packed_explicit(bg) {
            s = s.bg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(bg), ink.colors()));
        }
    }
    // ZMSD §8.7.1 style bits, as the model packs them (1 = reverse video,
    // 2 = bold, 4 = italic, 8 = fixed-pitch). Bold and italic used to be dropped
    // on every v6 cell path, so a game's emphasised menu text rendered roman.
    // They are ADDED when set (never removed): unlike REVERSED — which the
    // full-width flood rows below stamp into `base` and a non-reverse run must
    // clear — bold/italic only ever arrive from the run itself, so subtracting
    // them would fight the theme's own base style. Fixed-pitch (8) needs no
    // action in a monospaced terminal.
    //
    // Italic is `Modifier::ITALIC` — SGR 3, which asks the player's TERMINAL to draw
    // the run with its own italic face. §8.7.1 lets an interpreter interpret the bit
    // broadly ("rendering italic with underlining" is the standard's own example),
    // and the rule here is to use a real italic FACE where one is available and
    // underline where none is, never a slope we synthesised. On this path a face is
    // always available, because it is the terminal's. The path where lanthorn holds
    // the face itself is `render::bitfont` (SQ-1028).
    if style_bits & 2 != 0 {
        s = s.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if style_bits & 4 != 0 {
        s = s.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if style_bits & 1 != 0 {
        s.add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        s.remove_modifier(ratatui::style::Modifier::REVERSED)
    }
}

/// A status STRIP the game paints OVER the top of its own story window, rather than
/// above it (SQ-0582). advent.z6 leaves window 0 covering the whole 640×380 screen and
/// hangs window 1 — full width, one row tall, pinned at the top — over its first row.
/// Every other v6 game here reserves the band by placing the story window BELOW it
/// (Zork0 y=79, Shogun y=33, Arthur y=209), so the chrome ring picks the band up for
/// free; with an overlay there is no ring at all, and the bar's runs land inside the
/// story box to be stamped glyph-by-glyph over the transcript — a ribbon with holes
/// between the fields, and the transcript scrolling underneath it.
///
/// Returns the overlaying window, whose rows the caller reserves at the top of the
/// story viewport so the ordinary Text-strip path draws it as a solid bar.
fn overlaid_status_strip<'a>(
    chrome: &[&'a PositionedWindow],
    story: &PositionedWindow,
    native_w: u16,
    cell: zvm::screen::V6Cell,
) -> Option<&'a PositionedWindow> {
    let threshold = native_w as u32 * 9 / 10;
    chrome.iter().copied().find(|pw| {
        let WinNode::Grid(g) = &pw.node else { return false };
        // Text INSIDE the window's own rect. v6 text is paint and outlives the
        // window's geometry (ZMSD §8.8.4), so a window shrunk to a sliver while its
        // message box sits 50px lower — advent's own boot popup — is not a bar, and
        // reserving rows for it would inset the story out from under the popup.
        strip_rows(pw, g, cell).is_some()
            && pw.w_px as u32 >= threshold
            && pw.h_px > 0
            && pw.h_px <= V6_STATUS_STRIP_MAX_H_PX
            && pw.y_px <= story.y_px
            && pw.y_px.saturating_add(pw.h_px) > story.y_px
    })
}

/// The native rows a status strip's own text occupies: the last row carrying a
/// non-blank run that lies within the window's rect, or `None` when it paints none
/// there. (SQ-0582/SQ-0584)
fn strip_rows(pw: &PositionedWindow, g: &crate::engine::GridWindow, cell: zvm::screen::V6Cell) -> Option<u16> {
    let first = cell.row_of_origin0(pw.y_px);
    let last = pw.y_px.saturating_add(pw.h_px).div_ceil(cell.h()).max(first + 1);
    g.px_texts
        .iter()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| cell.row_of(t.y))
        .filter(|row| *row >= first && *row < last)
        .max()
}

/// Tallest v6 grid window that counts as a status STRIP for [`full_width_flood_rows`]:
/// two text rows. Matches `zvm::location`'s band rule, which mines the same shape for
/// the room name — a bar is one or two rows, anything taller is a panel or overlay.
const V6_STATUS_STRIP_MAX_H_PX: u16 = 32;

/// SQ-0515/SQ-0582: which native rows of a painted v6 screen should flood edge-to-edge
/// with a bar (see [`draw_painted_screen`]). A row qualifies only when each of its
/// non-blank runs belongs to a grid window spanning at least ~90% of the native screen
/// width — a full-width title/status bar (Zork0's " InvisiClues (tm)" header sits in a
/// w_px=640/640 window) rather than a narrow selection block (Shogun's boot-menu window
/// is w_px=169/640, and Zork0's own selected-topic highlight is in the w_px=468/640
/// topic window — both stay text-width) — AND one of:
///
///   - every run on it is reverse-video (the game asked for a bar), or
///   - the owning window is a STRIP at most [`V6_STATUS_STRIP_MAX_H_PX`] tall AND the
///     run lies INSIDE that window's own rect. Not every game styles its status line:
///     advent.z6 paints "At End Of Road … Score: 36 … Moves: 1" into a full-width,
///     one-row window with no reverse bit and no colours (SQ-0582), so the reverse
///     rule never fired and the theme's bar background reached only the cells under
///     the glyphs — a ribbon with holes between the fields. A window that shape IS the
///     status bar, whatever style its text carries.
///
///     The containment half is not pedantry: v6 text is PAINT, and a run stays where
///     it was put even when its window is later resized to nothing (ZMSD §8.8.4 — a
///     window's size "does not change the current display"). advent's own boot popup
///     is exactly that — window 1 shrunk to 640x1 while it paints a message box 50px
///     down the screen — so height alone called every popup row a status bar and
///     flooded the story text behind it edge to edge.
///
/// Returns native_row → flood [`Style`], with colours resolved first-explicit-wins
/// across the row and the reverse bit set only for the reverse case, via
/// [`v6_run_style`].
fn full_width_flood_rows(
    chrome: &[&PositionedWindow],
    native_w: u16,
    base: ratatui::style::Style,
    ink: TextInk,
    cell: zvm::screen::V6Cell,
) -> std::collections::HashMap<u16, ratatui::style::Style> {
    use crate::render::v6_layout::packed_explicit;
    let threshold = native_w as u32 * 9 / 10;
    // Group every non-blank run by native row, carrying its owning window's size.
    let mut per_row: std::collections::HashMap<u16, Vec<(&crate::engine::PxText, u16, u16, bool)>> = Default::default();
    for pw in chrome {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if t.text.trim().is_empty() {
                    continue;
                }
                // The row `draw_painted_screen` will PLACE this run on, so the
                // flood and the glyphs cannot land on different terminal rows
                // (SQ-1009): the run's own cell row, not its pixel row divided.
                let row = t.grow;
                // Does this run sit within the rows its own window covers?
                let win_first = cell.row_of_origin0(pw.y_px);
                let win_last = pw.y_px.saturating_add(pw.h_px).div_ceil(16).max(win_first + 1);
                let inside = row >= win_first && row < win_last;
                per_row.entry(row).or_default().push((t, pw.w_px, pw.h_px, inside));
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for (row, row_runs) in per_row {
        if !row_runs.iter().all(|(_, w, _, _)| *w as u32 >= threshold) {
            continue;
        }
        let all_reverse = row_runs.iter().all(|(t, _, _, _)| t.style & 1 != 0);
        let all_strip =
            row_runs.iter().all(|(_, _, h, inside)| *inside && *h > 0 && *h <= V6_STATUS_STRIP_MAX_H_PX);
        if !all_reverse && !all_strip {
            continue;
        }
        let fg = row_runs.iter().map(|(t, _, _, _)| t.fg).find(|&p| packed_explicit(p)).unwrap_or(0);
        let bg = row_runs.iter().map(|(t, _, _, _)| t.bg).find(|&p| packed_explicit(p)).unwrap_or(0);
        // Reverse only where the game asked for it: a plain strip floods with the
        // theme's own bar style, exactly as the anchored band does.
        let style_bits = u8::from(all_reverse);
        out.insert(row, v6_run_style(base, fg, bg, style_bits, ink));
    }
    out
}

/// Paint the background fields left by `erase_window` (SQ-0584).
///
/// ZMSD §8.8.5.3: erasing a window fills its rect with that window's background, and
/// on a real interpreter — where every v6 window is a clipping region over ONE screen
/// bitmap — that fill is opaque paint covering whatever was under it. A window carries
/// `fill` only while it is still the newest paint on its own rect (see
/// `GridWindow::fill`), so an ordinary turn, whose prose is newer, paints nothing here.
///
/// Drawn in the order the game erased them, over the whole pane rather than any one
/// draw call's row window: advent.z6's `help` erases the full screen and then its
/// 160px menu window, and the real screen that leaves is a menu panel on blank
/// background — not a panel with the transcript resuming under it.
/// Draw the v6 SECONDARY prose windows: flowing-text windows that are not the one
/// the player types into (SQ-0585). A v6 game may run several at once — advent.z6's
/// `style` opens one across the top of the screen and keeps playing in another below
/// — and the engine keeps each one's text in its own window rather than splicing them
/// into the transcript, so each draws in its own rect here.
///
/// Live screen state: what the window currently holds, no scrollback. Drawn after the
/// erase fills (a window is erased, then printed into) and before the chrome runs, so
/// a status bar painted over the same rows still lands on top.
fn draw_secondary_buffers(
    chrome: &[&PositionedWindow],
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    to_cells: &dyn Fn(&PositionedWindow) -> Rect,
) {
    for pw in chrome {
        let WinNode::Buffer(b) = &pw.node else { continue };
        if b.primary || b.lines.is_empty() {
            continue;
        }
        let r = to_cells(pw);
        let clipped = Rect::new(
            r.x.max(area.x),
            r.y.max(area.y),
            r.width.min(area.right().saturating_sub(r.x.max(area.x))),
            r.height.min(area.bottom().saturating_sub(r.y.max(area.y))),
        );
        if clipped.width == 0 || clipped.height == 0 {
            continue;
        }
        render_inline_buffer(b, state, clipped, buf);
    }
}

fn draw_erase_fills(
    chrome: &[&PositionedWindow],
    area: Rect,
    buf: &mut Buffer,
    base: ratatui::style::Style,
    ink: TextInk,
    to_cells: &dyn Fn(&PositionedWindow) -> Rect,
) {
    let mut fills: Vec<(&PositionedWindow, crate::engine::ErasedFill)> = chrome
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => g.fill.map(|f| (*pw, f)),
            _ => None,
        })
        .collect();
    fills.sort_by_key(|(_, f)| f.seq);
    for (pw, f) in fills {
        let style = v6_run_style(base, 0, f.bg, 0, ink);
        let r = to_cells(pw);
        for y in r.y.max(area.y)..r.bottom().min(area.bottom()) {
            for x in r.x.max(area.x)..r.right().min(area.right()) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    }
}

fn draw_painted_screen(
    runs: &[&crate::engine::PxText],
    rows: std::ops::Range<u16>,
    shift: i32,
    area: Rect,
    buf: &mut Buffer,
    base: ratatui::style::Style,
    ink: TextInk,
    chrome: &[&PositionedWindow],
    native_w: u16,
    cell: zvm::screen::V6Cell,
) {
    // A native row's terminal row, or `None` when it is outside this call's row
    // range or the shift pushes it off the pane.
    let place = |row: u16| -> Option<u16> {
        if !rows.contains(&row) {
            return None;
        }
        let y = area.y as i32 + row as i32 + shift;
        (y >= area.y as i32 && y < area.bottom() as i32).then_some(y as u16)
    };
    // SQ-0515: a native row whose non-blank runs are ALL reverse-video and all
    // belong to a (near-)full-native-width grid window is a title/status bar —
    // flood the whole terminal row edge to edge with the reversed style before
    // stamping the glyphs, so Zork0's " InvisiClues (tm)" header reads as a solid
    // reverse bar rather than reverse across only its own glyphs. A narrow window's
    // reverse row (Shogun's boot-menu selection) is untouched — it stays a
    // text-width highlight block below.
    let flood = full_width_flood_rows(chrome, native_w, base, ink, cell);
    for (&row, &style) in &flood {
        let Some(y) = place(row) else { continue };
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
    for t in runs {
        // Every run stamps, whitespace included — painter semantics. A reversed
        // space fills its cell of the selection bar (SQ-0484), and a NORMAL
        // space must equally repaint over an earlier reversed one: when the
        // menu selection moves, the game repaints the old row's gaps as plain
        // spaces, and skipping those left the stale reversed cells behind
        // (SQ-0490).
        // The CELL the engine wrote this run's first glyph in (SQ-1009), never
        // the run's pixel origin divided by the declared cell width. The division
        // is the column only while the pen advances one declared cell per
        // character; at Arthur's ~10.4 native pixels against a declared 8 it
        // climbs 1.3 per glyph, so a derived column skips cells and the drift
        // compounds along the line — `Churchyard` reads `Ch urc  hy ard`, and a
        // wider pane makes it worse. For every fixed-pen machine this IS the
        // division, so nothing else moves.
        let row = t.grow;
        let Some(y) = place(row) else { continue };
        let col = t.gcol;
        if area.x + col >= area.right() {
            continue;
        }
        // Cell styles PATCH — a repaint must explicitly clear the reverse bit,
        // or a cell once reversed stays reversed after the game repaints it
        // plain (SQ-0490). Explicit game colours on the run replace the theme
        // base per channel; inherited/Default channels keep it (SQ-0488).
        let style = v6_run_style(base, t.fg, t.bg, t.style, ink);
        let max_w = (area.right() - (area.x + col)) as usize;
        // Untrusted game text (SQ-0639): a control char would shift the rest of
        // the run a column left, and these runs are pixel-placed.
        let text = crate::render::blank_control_chars(&t.text);
        buf.set_stringn(area.x + col, y, text.as_ref(), max_w, style);
    }
}

/// One horizontal strip of a full-width hybrid chrome band (SQ-0500): either
/// `Art` (opaque frame graphics behind it — keep the scaled pixel ring) or `Text`
/// (no graphics behind, only status/menu runs — paint as terminal cells). The
/// runs carried by a `Text` strip are the chrome grid runs that map into it.
enum ChromeStrip {
    /// A run of pixels from the chrome canvas, tagged with which part of the ring
    /// it belongs to (SQ-0894). Only `Art` needs the tag: a `Text` strip is never a
    /// flank, because a flank is emitted whole as one `Art` strip and never split
    /// row-by-row.
    ///
    /// The tag is what stops the downstream stages measuring. Ten of them asked
    /// `r.width < area.width` (or `>=`) to mean "is this a flank", which is true
    /// only while the top and bottom bands span the pane — exactly the definition
    /// the content-built ring replaces. With top/bottom now narrowed to the
    /// viewport's columns on the rows a flank took, a width test reads a narrowed
    /// TOP band as a flank, which is how Journey's Amiga press grew a third
    /// `flank-divider` beside its real one.
    Art(crate::render::v6_layout::BandRole, Rect),
    Text(Rect, Vec<crate::engine::PxText>),
}

/// SQ-0505 dynamic hybrid layout: how the vertical letterbox slack below the
/// story window is reclaimed.
///   `Letterbox` — keep today's centred frame (Zork0's enclosed full frame, or a
///                 pane with no slack to reclaim).
///   `Extend`    — top-anchor the ring and grow the story viewport to the pane
///                 bottom (Arthur: header art + side borders, open below).
///   `Menu`      — top-anchor the story/chrome and bottom-anchor a text command
///                 strip; the story fills between (Journey's command menu).
///   `Frame`     — top-anchor the story and grow it to the pane bottom, but keep
///                 the ENCLOSING side art by stretching the flank bands vertically
///                 to span the reclaimed space (SQ-0511: Zork0/Shogun, whose frame
///                 reaches the native bottom and is flanked by full-height side art).
enum BottomPlan {
    Letterbox,
    Extend,
    Menu,
    Frame,
}

/// SQ-0570: is this frame a full-screen PICTURE takeover — a picture painted
/// across the whole screen with the story window grown over it?
///
/// Zork Zero's `map` command is the case. It is the exact inverse of the title
/// splash: the splash calls `split_window(400)` so window 1 becomes the screen and
/// window 0 COLLAPSES to zero height, leaving no story viewport to carve (SQ-0497),
/// whereas the map GROWS window 0 to the full screen `(0,0) 640×400` and paints the
/// map into the full-screen graphics window beneath it. Hybrid mode then made the
/// story viewport the entire pane, which leaves `chrome_bands` empty — so the map
/// was never uploaded at all and the transcript painted over the whole screen. (On
/// screen it reads as the frame falling away mid-game: no frame, no picture, just
/// text.)
///
/// Such a frame has no ring to draw, so there is nothing for hybrid to do: the
/// caller falls through to the RASTER composite, which draws the picture and
/// rasterizes the story text over it in one canvas, and already renders this screen
/// correctly. Detection is deliberately narrow — the story window must cover the
/// whole screen (within one native text row per edge) AND opaque graphics must sit
/// behind it, either FILLING it or FRAMING it (below). An ordinary gameplay screen
/// keeps window 0 inset inside its frame, so it can never qualify.
///
/// The opacity test samples a coarse grid rather than every pixel: it runs on every
/// hybrid frame, and a fully painted picture versus a frame-only (or empty) canvas
/// is not a close call.
///
/// SQ-0729: filling the screen was too strong a test. fmvpoker paints a 640×400
/// poker table into full-screen window 0 and prints its whole title inside it, and
/// that table is a FRAME — 17% of its pixels opaque, the middle a hole — so the
/// grid below misses it at every point that matters and hybrid kept its (empty)
/// ring. The game drew not one picture on screen. What actually decides this is
/// the story window covering the screen: that alone leaves `chrome_bands` with
/// nothing to carve, so NO art behind it can be uploaded whatever its shape. So a
/// second arm asks whether the art ENCLOSES the screen instead — painted pixels
/// within a native text row of all four edges, which is "the painted bounding box
/// spans the screen" without scanning the interior. Measured across the v6 corpus,
/// this moved fmvpoker alone: Zork Zero and Shogun keep window 0 inset, advent and
/// scopa paint nothing behind it, Arthur's intro plate and Journey's title already
/// fill the screen, and mysterious01's plate reached neither the right edge nor
/// (at the time) the top one. That last exception is the SQ-0725 arm below.
///
/// SQ-0739 ADDED A FOURTH ARM AND SQ-0897 RETIRED IT. `story_plate_escapes_story_window`
/// asked whether the story window's plate painted outside the window it is the plate
/// OF — the frame where fmvpoker's bottom panel became the window the game read
/// through while window 0 still held the 640x400 poker table, so the art was in
/// neither the ring nor the viewport. Two things killed it, and either would have
/// been enough:
///
/// * SQ-0746 removed its premise. Reading through a panel the game declares is NOT
///   its transcript (attribute 2 clear) never made that panel the story window, so
///   window 0 never stops being it and the plate never escapes anything. The frame
///   is carried by the enclosure arm below, exactly as at the menu.
/// * SQ-0896 removed its need. The ring's canvas and its art oracle now carry the
///   story window's own plate at its own native origin, wherever that is, and the
///   viewport is cut from what the plate LEAVES — so a plate reaching outside the
///   window lands in the ring like any other pixel.
///
/// MEASURED before removal, `ring_scout --all --no-tap --turns 8` at a 98x37 pane:
/// the predicate answered false on every frame of all eight titles. Its own corpus
/// test (`no_corpus_plate_escapes_its_story_window`) had asserted exactly that since
/// SQ-0746, and the bet-panel case it was written for asserted it on that very frame.
///
/// SQ-0725 GENERALISED THE FIRST TWO ARMS INTO ONE. Both were proxies for a fact
/// the premise already states outright: `blit_story_gfx` is reachable from the
/// RASTER path alone, so once the story window covers the screen — no ring — a
/// plate that goes anywhere but the composite goes nowhere. "Fills" and
/// "encloses" were two shapes that happen to satisfy that; they were never the
/// reason. mysterious01 is the title that is neither shape and still has no ring:
/// its boot stacks two 512×192 title cards down the left of a 640×400 screen, so
/// the art misses the fill grid on the right-hand quarter and misses the
/// enclosure test on the right edge, and hybrid — the shipped default — drew
/// **not one pixel of either card**. Only the card the SQ-0722 misclassification
/// smuggled down the transcript-float path was ever visible, which is why the two
/// quests read as separate bugs.
///
/// So the third arm is the premise itself: a full-screen story window whose plate
/// paints ANYTHING must take the composite. Measured over the corpus, every frame
/// that has both a full-screen story window and a painted plate already took
/// raster by one of the old arms — Arthur's intro and Journey's title at 64/64
/// probe points, fmvpoker's hollow frame at 4–10/64 — and mysterious01 (48/64 at
/// boot, 24/64 after) is the sole frame the generalisation moves. There is no
/// corpus frame where a full-screen story window has a painted plate and the ring
/// is the right answer, so this is a categorical rule and not a tuned threshold.
///
/// The tradeoff is real and deliberate: a frame that stops taking the ring renders
/// as a pixel image instead of crisp terminal cells. It is still the right trade,
/// because the alternative for such a frame is not crisper art — it is no art.
///
/// ## SQ-0897: the three surviving arms, and why each survives
///
/// SQ-0896 gave the ring somewhere to put these pixels — the viewport is cut from
/// what the art LEAVES, so art the game painted inside window 0 lands in the ring
/// like any other pixel. That is the precondition for retiring these, and one of
/// them went (see the SQ-0739 paragraph above). The other three were driven on the
/// fixtures they were written for and KEPT. Each reason is a measurement, not a
/// judgement about tidiness.
///
/// **`art_paints_anything` (SQ-0725) — kept. The ring reads a title card as a
/// border rule.** Retiring it moves exactly ONE title: MEASURED with the arm
/// disabled, `ring_scout --all --no-tap --turns 8`, Arthur's intro plate and
/// Journey's title fall through to `art_fills_screen` and every fmvpoker frame to
/// `art_encloses_screen`, so only mysterious01 changes route — and mysterious01 is
/// the title this arm was written for. What the ring then draws is wrong. Its two
/// 512x192 cards stack down the left of a 640x400 screen, so the content-carved ring
/// makes them ONE 79x37 side flank over the whole pane, and a flank goes through the
/// border-extension recipe: the band log reads `[Art, tiled] source 516x544 native
/// px` for 384 rows of artwork, i.e. 160 native rows of picture invented below the
/// second card, drawn across the letterbox margins the Letterbox plan meant to leave
/// bare. Extending a picture column as though it were a side rule is exactly the
/// mistake SQ-0819 excluded the Menu plan from `tiled_flanks` to avoid — Journey's
/// picture column is not a border either. Retiring this arm needs the ring to tell a
/// picture column from a border column; that is a change to `v6_border` and
/// `tiled_flanks`, not to this router, and it must land first.
///
/// **`art_fills_screen` — kept, and it is not shadowed by the arm above.** It decides
/// nothing on today's corpus, because every frame whose art fills the screen has that
/// art on window 0's own plate and the first arm answers for it. But this arm reads
/// CHROME art too, which the first never looks at, and a full-screen chrome BACKDROP
/// under a full-screen story window is the one shape SQ-0896 deliberately declines:
/// its inset floor hands such a window its declared box back rather than let four
/// converging edges eat a story region, so the viewport is the whole pane, the ring
/// is empty, and without this arm the backdrop would not be drawn at all. Retiring it
/// would reintroduce SQ-0570's original symptom on the one shape SQ-0896 does not
/// cover. Separately, for a full-bleed PLATE there is nothing for the ring to add:
/// such a frame carries no text to keep crisp (`story_prose_box` returns `None` and
/// hybrid renders no transcript on it), so the ring would ship the same picture in N
/// band uploads that raster ships in one image.
///
/// **`art_encloses_screen` (SQ-0729) — kept, because it is not only a routing test.**
/// [`story_window_is_a_canvas`] reuses this exact predicate, deliberately, so the two
/// cannot drift — and the CANVAS reading of window 0 (its live runs painted at the
/// coordinates the game's own `set_cursor` named, with no transcript at all) exists
/// on the raster path alone, in `draw_story_canvas_runs`. Route fmvpoker to the ring
/// and it gets a scrolling re-render of everything window 0 ever printed, wrapped
/// into the 594x158 hole in its poker table — which is the reading SQ-0729 measured
/// as wrong, and how its dealt hand came to overwrite the line the player needs.
/// Retiring this arm needs the canvas reading on the ring path first.
///
/// `picture_takeover_arms_across_the_corpus` pins the census these verdicts rest on.
/// WHICH hatch fires, for instruments and tests (SQ-0897).
///
/// Retiring these one at a time needs a way to say "this frame is here because of
/// THIS arm", and a boolean cannot. Four OR'd predicates over the same frame are
/// not four separate facts on the corpus — `art_paints_anything` subsumes both of
/// the shapes below it — so "the gate is closed" says nothing about which arm shut
/// it. Ordered as the boolean evaluates, first match wins, so the name is the arm
/// that actually decided.
///
/// `ring_scout` prints this on every frame it scouts. Reachability is not something
/// to infer from a green gate: SQ-0894 disabled an arm, passed all 5553 tests, and
/// the arm was live — the corpus simply had no fixture that reached it.
pub fn picture_takeover_reason(
    story: &crate::engine::PositionedWindow,
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> Option<&'static str> {
    if story_covers_screen(story, native) {
        if art_paints_anything(story_gfx, native) {
            return Some("art_paints_anything");
        }
        if art_fills_screen(chrome, story_gfx, native) {
            return Some("art_fills_screen");
        }
        if art_encloses_screen(chrome, story_gfx, native) {
            return Some("art_encloses_screen");
        }
    }
    None
}

/// Does the story window's own plate paint anything at all (SQ-0725)?
///
/// Sampled on the same coarse 8×8 grid as [`art_fills_screen`], for the same
/// reason — this runs on every hybrid frame, and a plate with art on it versus an
/// empty canvas is not a close call. Only `story_gfx` is asked: CHROME art has a
/// ring to live in and is none of this arm's business.
fn art_paints_anything(
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    const N: u32 = 8;
    let painted = art_painted_probe(&[], story_gfx);
    (0..N).any(|iy| {
        let y = native.1 as u32 * (2 * iy + 1) / (2 * N);
        (0..N).any(|ix| {
            let x = native.0 as u32 * (2 * ix + 1) / (2 * N);
            painted(x, y)
        })
    })
}

/// One native text row of slack per edge, so a game that leaves a hairline border
/// still counts as covering the screen.
const SCREEN_SLOP: u32 = 16;

/// Is a pixel painted by any of these graphics windows?
fn art_painted_probe<'a>(
    chrome: &'a [&'a crate::engine::PositionedWindow],
    story_gfx: Option<&'a crate::engine::PositionedWindow>,
) -> impl Fn(u32, u32) -> bool + 'a {
    let painted_at = |x: u32, y: u32, pw: &crate::engine::PositionedWindow| {
        let crate::engine::WinNode::Graphics(gw) = &pw.node else { return false };
        let (wx, wy) = (pw.x_px as u32, pw.y_px as u32);
        let img = &gw.canvas;
        x >= wx
            && y >= wy
            && x - wx < img.width()
            && y - wy < img.height()
            && img.get_pixel(x - wx, y - wy)[3] >= 128
    };
    move |x: u32, y: u32| {
        chrome.iter().any(|pw| painted_at(x, y, pw)) || story_gfx.is_some_and(|pw| painted_at(x, y, pw))
    }
}

/// Does the artwork FILL the screen — a solid plate the game narrates over?
///
/// Sampled on a coarse 8×8 grid rather than every pixel: this runs on every hybrid
/// frame, and a fully painted picture versus a frame-only (or empty) canvas is not
/// a close call. The STORY window's own plate counts, not just chrome: Arthur's
/// intro erases every window, centres a 584×392 illustration inside full-screen
/// window 0 and narrates over it (SQ-0695), and there is no chrome ring at all on
/// those screens — scanning chrome alone found nothing painted and hybrid opened a
/// pane-wide transcript viewport over art it then never uploaded.
fn art_fills_screen(
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    const N: u32 = 8;
    let painted = art_painted_probe(chrome, story_gfx);
    (0..N).all(|iy| {
        let y = native.1 as u32 * (2 * iy + 1) / (2 * N);
        (0..N).all(|ix| {
            let x = native.0 as u32 * (2 * ix + 1) / (2 * N);
            painted(x, y)
        })
    })
}

/// Does this story window cover the whole screen (within [`SCREEN_SLOP`] per edge)?
/// Such a window leaves hybrid's `chrome_bands` with nothing to carve.
fn story_covers_screen(story: &crate::engine::PositionedWindow, native: (u16, u16)) -> bool {
    (story.x_px as u32) <= SCREEN_SLOP
        && (story.y_px as u32) <= SCREEN_SLOP
        && story.x_px as u32 + story.w_px as u32 + SCREEN_SLOP >= native.0 as u32
        && story.y_px as u32 + story.h_px as u32 + SCREEN_SLOP >= native.1 as u32
}

/// Does the artwork ENCLOSE the screen (SQ-0729) — painted pixels within one native
/// text row of every edge? Probed edge strip by edge strip, so a hollow FRAME
/// answers on its border instead of failing on its hole, which is what makes this
/// different from "the art fills the screen".
///
/// Corpus-measured when it was written, and the measurement is what makes it safe
/// to reuse: it fires for exactly one title, fmvpoker. Zork Zero and Shogun keep
/// window 0 inset inside their frames, advent and scopa paint nothing behind it,
/// Arthur's intro plate and Journey's title are solid and answer the FILL test
/// instead, and mysterious01's plate is a 512×192 band across the lower half that
/// reaches neither the top edge nor the right one.
fn art_encloses_screen(
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    let painted = art_painted_probe(chrome, story_gfx);
    let (w, h) = (native.0 as u32, native.1 as u32);
    let any_painted = |xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
        ys.step_by(2).any(|y| xs.clone().step_by(2).any(|x| painted(x, y)))
    };
    any_painted(0..w, 0..SCREEN_SLOP)
        && any_painted(0..w, h.saturating_sub(SCREEN_SLOP)..h)
        && any_painted(0..SCREEN_SLOP, 0..h)
        && any_painted(w.saturating_sub(SCREEN_SLOP)..w, 0..h)
}

/// SQ-0729: is this story window a CANVAS rather than a page — a window the game
/// has drawn a frame AROUND and then positions text INSIDE, rather than a
/// transcript it narrates on?
///
/// The discriminator is deliberately not "what does this RUN mean". A `set_cursor`
/// before a run is genuinely ambiguous: Arthur positions every room headline in
/// window 0 (one character at a time, only the first carrying the declaration),
/// Shogun and Journey centre each header line the same way, and mysterious01
/// re-homes before its prompt — all of them meaning "resume the transcript here",
/// while fmvpoker's HOLD means "paint this under that card". Nothing in the signal
/// separates them, which is what a measured attempt at that rule established.
///
/// So the question asked here is what kind of SURFACE the window is. Arthur's
/// window 0 is a transcript that happens to have plates drawn on it; fmvpoker's is
/// a picture frame that happens to have text positioned in it — its own art
/// encloses it on all four sides and it covers the whole screen. That is the same
/// test [`picture_takeover_reason`]'s enclosure arm asks, reused rather than restated so
/// the two cannot drift apart, and it fires for fmvpoker alone.
///
/// It also extends a rule this codebase already made: SQ-0711/SQ-0716 ruled that a
/// window the game has drawn into is a canvas and keeps the ground it drew on.
/// This says the same of the text on that ground.
///
/// ENCLOSING and not FILLING is the whole discriminator, and both halves are
/// load-bearing. A solid full-screen plate reaches all four edges too, so Journey's
/// title read as a canvas until the fill test excluded it — and a plate is a
/// picture a game NARRATES OVER (Arthur's illustrated screens, Journey's title),
/// while a frame with a hole in the middle is a picture a game POSITIONS TEXT
/// INSIDE. [`picture_takeover_reason`] takes either, because for its purposes — hybrid has
/// no ring to draw — the two are the same.
pub fn story_window_is_a_canvas(
    layout: &crate::render::v6_layout::V6Layout<'_>,
    native: (u16, u16),
) -> bool {
    layout.story.is_some_and(|s| story_covers_screen(s, native))
        && !art_fills_screen(&layout.chrome, layout.story_gfx, native)
        && art_encloses_screen(&layout.chrome, layout.story_gfx, native)
}

/// Classify what sits below the story window natively, to pick the [`BottomPlan`]
/// (SQ-0505). `slack` is the vertical letterbox margin in device pixels.
///
/// Keeps the centred letterbox when there is no slack to reclaim, when the story
/// already reaches the native screen bottom (its frame encloses it — Zork0's story
/// bottom is 398 of 400), or when a real ART band spans the story columns below it
/// (rule 4). Otherwise the below-story region is text-only (→ `Menu`) or empty
/// (→ `Extend`). The art test is restricted to the STORY COLUMNS so full-height
/// side borders (which flank, not floor, the story) never read as a bottom band.
///
/// SQ-0830: **`Menu` is decided before slack is, because a command menu is a fact
/// about the FRAME and slack is a fact about the pane.** The `slack == 0` shortcut
/// used to come first, so any pane whose vertical axis is the binding letterbox
/// axis stopped recognising Journey's menu as a menu at all — and everything
/// gated on `plan == Menu` went out together: no `menu_flank_panel` (so no panel
/// fill sampled from the art's own edge, no vertical centring, no aspect-correct
/// dest box), no exclusion from `tiled_flanks` (so the picture column fell through
/// to the side-border TILER, which SQ-0819 established is exactly wrong for a
/// picture seated in a panel), and `glyph_borders_only` flipped true. The user's
/// own 166x44 is one such pane: the v6 area is 164x41 cells = 1312x738 device px
/// at an 8x18 cell, s = min(1312/640, 738/400) = 1.845 exactly, slack 0.
///
/// Slack now gates only the RECLAIM, which is all it was ever about — and a Menu
/// plan at zero slack degrades to "menu, no reclaim" for free rather than needing
/// an arm of its own: the plan's `menu` scale is `off_y = slack`, and `Letterbox`'s
/// centred scale is `off_y = slack / 2`, so at `slack == 0` both are the
/// top-anchored scale and no band moves. Only the flank TREATMENT changes, which
/// is the whole of the defect.
///
/// The hoist is safe against the arms below it because [`menu_strip_below_story`]
/// carries their guard itself: it is false as soon as the story reaches within a
/// native text row of the screen bottom — or, since SQ-1157, as soon as what lies
/// below it is a band this frame carries — which is precisely when the enclosed-frame
/// arm fires. Measured across the corpus, this moves Journey (both releases) and
/// nothing else — Arthur reads no menu at any pane, and Shogun and Zork Zero are
/// enclosed frames that never get as far as asking.
fn hybrid_bottom_plan(
    story: &crate::engine::PositionedWindow,
    gfx: &image::RgbaImage,
    // SQ-1157: the chrome WINDOWS, not the runs pulled out of them. The question
    // this asks is about a band's window — can the frame move it as a unit? — and a
    // list of runs cannot answer it. The runs are derived where they are needed.
    chrome: &[&crate::engine::PositionedWindow],
    native: (u16, u16),
    slack: u32,
    // See `menu_strip_below_story` — the same "one native row" question.
    cell: zvm::screen::V6Cell,
) -> BottomPlan {
    if menu_strip_below_story(story, gfx, chrome, native, cell) {
        return BottomPlan::Menu;
    }
    if slack == 0 {
        return BottomPlan::Letterbox;
    }
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    // How far the game's own content reaches DOWN (SQ-1157): the story window's
    // bottom, or — where the game anchored a band of whole windows under it — that
    // band's. Arthur's parser-error window 3 is the band, and its height is
    // TRANSIENT: 584x16 across native row 384 for a message that fits, 584x**32**
    // across 368 for one that wraps ("Sorry, but I don't understand. Please rephrase
    // that, or try something else." — the `was` repro). Measuring the story window
    // alone made a two-row message a different KIND of frame from a one-row one:
    // `native.1 (400) <= 384 + 16` holds and `<= 368 + 16` does not, so the enclosed
    // frame that stretches Arthur's poles became a `Menu`, the plan meant for
    // Journey's command strip. The band travels with the frame's bottom edge either
    // way (the `Extend | Frame` arm reserves exactly its rows off the pane), so what
    // the frame has to reach is the band's bottom and not the story's.
    let reach = anchored_band_bottom(chrome, story, native).unwrap_or(story_bottom);
    // Story fills to (within one native row of) the screen bottom → enclosed frame.
    // SQ-0511: when full-height side ART flanks the story on BOTH sides, reclaim the
    // slack via the `Frame` plan (top-anchor the story to the pane bottom, stretch
    // the flanks to keep the enclosing columns). Zork0 (story bottom 398/400) and
    // Shogun (400/400) both qualify. With no side art there is nothing to stretch, so
    // keep the centred letterbox.
    if native.1 as u32 <= reach + u32::from(cell.h()) {
        let sy0 = story.y_px as u32;
        let sy1 = story_bottom.min(gfx.height());
        let sx0 = story.x_px as u32;
        let sx1 = (story.x_px as u32 + story.w_px as u32).min(gfx.width());
        let flank_opaque = |xa: u32, xb: u32| -> bool {
            xa < xb && (sy0..sy1).any(|y| (xa..xb).any(|x| gfx.get_pixel(x, y)[3] >= 128))
        };
        let left_art = flank_opaque(0, sx0.min(gfx.width()));
        let right_art = flank_opaque(sx1, gfx.width());
        if left_art && right_art {
            return BottomPlan::Frame;
        }
        // SQ-0571: with no enclosing side art there is nothing to stretch, so
        // top-anchor (`Extend`) rather than CENTRE the frame. Centring made the
        // whole screen's position depend on the story window's height, and Arthur
        // changes that height mid-game: `map` grows win0 from 128 to 192 native px
        // (bottom 400), and its F6 text screen opens win0 at 640×384 (bottom 400),
        // both of which flipped the plan Extend → Letterbox. The centred offset then
        // dropped everything — header art, the map drawn into it, or a bare text
        // page — half the letterbox slack down the pane, and dismissing the screen
        // shrank the window and jumped it all back to the top. `Extend` simply lets
        // the story fill to the pane bottom, exactly as it does at the smaller
        // window height, so nothing moves. Zork0/Shogun are unaffected: their
        // full-height side art takes the `Frame` arm above.
        return BottomPlan::Extend;
    }
    // SQ-0830: the `Menu` arm used to live here. It is now the first question the
    // function asks, so what is left below the story is empty by elimination.
    BottomPlan::Extend
}

/// Does a TEXT-ONLY strip of the game's own chrome sit below the story window?
///
/// This is the whole of [`BottomPlan::Menu`] — Journey's command menu, the one
/// arm of [`hybrid_bottom_plan`] that owes nothing to the pane. It is factored
/// out because RASTER needs the same answer and has no pane to ask about
/// (SQ-0819): the raster composite is built in native pixels, so `slack` and the
/// letterbox arms above are meaningless to it, but "what lives under the story
/// window" is a property of the FRAME and both modes must read it the same way.
///
/// False as soon as the story reaches (within one native text row of) the screen
/// bottom — there is no strip below it to find — and false, since SQ-1157, when what
/// IS down there is a band of whole chrome windows the frame carries with its own
/// bottom edge ([`anchored_band_bottom`]). That guard is the function's own,
/// not the caller's, and since SQ-0830 that matters: [`hybrid_bottom_plan`] asks
/// this FIRST, ahead of its enclosed-frame and zero-slack arms, so this test is
/// what keeps Zork Zero's and Shogun's enclosed frames out of the `Menu` plan.
fn menu_strip_below_story(
    story: &crate::engine::PositionedWindow,
    gfx: &image::RgbaImage,
    // SQ-1157: the chrome WINDOWS. "Is this strip Journey's menu?" is a question
    // about the window the runs belong to, and the runs alone cannot answer it —
    // see the anchorable arm below.
    chrome: &[&crate::engine::PositionedWindow],
    native: (u16, u16),
    // SQ-1020: "one native ROW of slack" is a question about the GAME's cell, and
    // was written `+ 16` here — right on every machine whose cell is 16 and quietly
    // wrong on the one whose is not. This function had no cell to ask, which is why
    // it survived SQ-0917's sweep; it takes one now.
    cell: zvm::screen::V6Cell,
) -> bool {
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    if native.1 as u32 <= story_bottom + u32::from(cell.h()) {
        return false;
    }
    let sx0 = story.x_px as u32;
    let sx1 = (story.x_px as u32 + story.w_px as u32).min(gfx.width());
    let colw = sx1.saturating_sub(sx0);
    // A genuine bottom ART band covers most of the story columns below the window.
    let art_band = colw > 0
        && (story_bottom..native.1 as u32).any(|y| {
            let cnt = (sx0..sx1).filter(|&x| gfx.get_pixel(x, y)[3] >= 128).count() as u32;
            cnt * 2 >= colw
        });
    if art_band {
        return false;
    }
    // SQ-1157: …and neither is a band the frame can carry as WHOLE WINDOWS. That is
    // the distinction this function was making all along, by proxy: Arthur's
    // parser-error window 3 lies entirely between the story window's bottom and the
    // screen's, so it moves with the frame's bottom edge; Journey's menu runs belong
    // to a full-screen `(0,0) 640x400` grid straddling the story, which nothing can
    // move without moving the whole screen. The proxy was the "within one native
    // text row" guard above, and it only ever forgave a ONE-row band — so Arthur's
    // OWN band read as Journey's menu the moment a parser message wrapped to two
    // rows, and the frame changed shape on the length of an error string.
    if anchored_band_bottom(chrome, story, native).is_some() {
        return false;
    }
    paint_runs(chrome)
        .any(|t| !t.text.trim().is_empty() && (t.y.max(1) as u32 - 1) >= story_bottom)
}

/// Every paint run carried by a list of chrome windows, in window order.
///
/// The one place `chrome` is turned into runs, so a question asked of the runs and a
/// question asked of the windows are asked of the same set (SQ-1157).
fn paint_runs<'a>(
    chrome: &'a [&'a crate::engine::PositionedWindow],
) -> impl Iterator<Item = &'a crate::engine::PxText> {
    chrome
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
}

/// How far down the game's own bottom-anchored band reaches, in native pixels —
/// `None` when there is no such band, either because nothing lies below the story
/// window or because what does cannot be moved as whole windows (SQ-1157).
///
/// [`bottom_anchored_chrome`] is the test; this is its answer stated as a row, which
/// is what both [`menu_strip_below_story`] and [`hybrid_bottom_plan`] need. An empty
/// window list — nothing below the story at all — is `None`, so a caller's
/// `unwrap_or(story_bottom)` is the story window's own bottom, exactly as before.
fn anchored_band_bottom(
    chrome: &[&crate::engine::PositionedWindow],
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
) -> Option<u32> {
    bottom_anchored_chrome(chrome, story, native)?
        .into_iter()
        .map(|i| u32::from(chrome[i].y_px) + u32::from(chrome[i].h_px))
        .max()
}

/// SQ-0698: the native geometry a side flank band occupies — its columns
/// `[x0, x1)`, the native row its top maps back to, and how many native rows its
/// device height is worth at the UNIFORM scale.
///
/// It had a sibling, `flank_crop`, which read the same band back through the same
/// scale and then ran its rows down to the ARTWORK's bottom however tall the band
/// was — the source for SQ-0511's Frame-plan stretch. SQ-0898 removed that arm, and
/// this is now the only way a flank band's native box is derived.
///
/// **And where that native box LANDS**, in device pixels relative to the band's
/// own top-left ([`FlankBox::dest`]). The two travel together because they are one
/// decision: a native span and the device span it occupies at the frame's scale.
/// Returning only the native half is what shipped SQ-0898 — the caller clipped the
/// source to the art that exists and handed it to an unclipped destination, so the
/// resize absorbed the difference as a change of magnification. See
/// [`crate::render::graphics::GraphicsRender::draw_chrome_band_image`] for the
/// measurement.
struct FlankBox {
    /// Native columns `[x0, x1)` of the game's screen this band shows.
    x0: u32,
    x1: u32,
    /// The native row the band's visible top maps back to, and how many native
    /// rows it is worth at `scale.s`. `top + rows` may run PAST the game's screen:
    /// that is the flank extension, and supplying more rows of art is the one thing
    /// a tiled flank may do that a crop may not.
    top: u32,
    rows: u32,
    /// Where `[x0, x1) × rows` belongs inside the band, in device pixels from the
    /// band's top-left. `w ≈ (x1 - x0) · s` and `h ≈ rows · s`, so drawing the
    /// source into it is the frame's own magnification by construction.
    dest: crate::render::graphics::BandDest,
}

fn flank_native_box(
    band: Rect,
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    native: (u16, u16),
) -> FlankBox {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    let s = if scale.s <= 0.0 { 1.0 } else { scale.s };
    // The band's own device span, and the span the game's scaled screen occupies
    // inside the pane — the same `off + round(native · s)` box the plain crop reads
    // its pixels out of, so the two agree on where the artwork's edge is.
    let rel_x0 = band.x.saturating_sub(pane.x) as f32 * cw;
    let rel_y0 = band.y.saturating_sub(pane.y) as f32 * ch;
    let (bw, bh) = (band.width as f32 * cw, band.height as f32 * ch);
    let art_x0 = scale.off_x as f32;
    let art_x1 = art_x0 + (native.0 as f32 * s).round();
    let art_y0 = scale.off_y as f32;
    // Horizontally the flank is bounded by the artwork: there are no native columns
    // outside the game's screen, so the letterbox margin beside it is margin, and
    // stretching into it is what SQ-0898 was. Vertically it is bounded only ABOVE:
    // below, the per-title recipe generates as many rows as the band asks for, which
    // is what "tiled, not stretched" means and why this costs nothing to satisfy.
    let dx0 = rel_x0.max(art_x0);
    let dx1 = (rel_x0 + bw).min(art_x1).max(dx0);
    let dy0 = rel_y0.max(art_y0);
    let dy1 = (rel_y0 + bh).max(dy0);
    let x0 = ((dx0 - art_x0) / s).round().max(0.0) as u32;
    let x1 = (((dx1 - art_x0) / s).round().max(0.0) as u32).min(native.0 as u32).max(x0);
    let top = ((dy0 - art_y0) / s).round().max(0.0) as u32;
    let rows = ((dy1 - dy0) / s).round().max(0.0) as u32;
    FlankBox {
        x0,
        x1,
        top,
        rows,
        dest: (
            (dx0 - rel_x0).round().max(0.0) as u32,
            (dy0 - rel_y0).round().max(0.0) as u32,
            (dx1 - dx0).round().max(0.0) as u32,
            (dy1 - dy0).round().max(0.0) as u32,
        ),
    }
}

/// SQ-0698: which border layout — if any — this side flank band is showing.
///
/// Measured on the GRAPHICS-ONLY canvas, so a status run rasterised into the
/// chrome canvas can never be mistaken for border art (the same canvas
/// `strip_has_art` asks, for the same reason).
fn flank_border_art(
    band: Rect,
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    native: (u16, u16),
    gfx: &image::RgbaImage,
) -> Option<crate::render::v6_border::BorderArt> {
    let b = flank_native_box(band, pane, scale, cell_px, native);
    if b.x1 <= b.x0 {
        return None;
    }
    let art = crate::render::v6_border::art_extent(gfx, b.x0, b.x1);
    crate::render::v6_border::recognize(gfx, b.x0, b.x1, art, native.1 as u32)
}

/// The pixels one tiled flank band ships, and where inside the band they go
/// (SQ-0898). They travel together because the pair IS the magnification.
type TiledFlank = (image::RgbaImage, crate::render::graphics::BandDest);

/// SQ-0698: the tiled native source for one side flank band, or `None` when its
/// art is unrecognised or already covers the band.
///
/// Sampled from the CHROME canvas (the pixels a band ships) but classified from
/// the GRAPHICS-ONLY canvas (what is artwork).
fn flank_tiled_source(
    band: Rect,
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    native: (u16, u16),
    canvas: &image::RgbaImage,
    gfx: &image::RgbaImage,
) -> Option<TiledFlank> {
    let b = flank_native_box(band, pane, scale, cell_px, native);
    if b.x1 <= b.x0 || b.rows == 0 {
        return None;
    }
    let art = crate::render::v6_border::art_extent(gfx, b.x0, b.x1);
    // The destination rides along with the pixels: the source is `[x0, x1) × rows`
    // of native art and `dest` is exactly where that lands at the frame's scale, so
    // the draw has no size left to choose (SQ-0898).
    crate::render::v6_border::flank_source(canvas, gfx, b.x0, b.x1, art, native.1 as u32, b.top, b.rows)
        .map(|img| (img, b.dest))
}

/// SQ-0793: extend the side border art down the RASTER composite, in place.
///
/// The hybrid ring gained this in SQ-0698 and raster did not, so the two modes
/// disagreed on the same frame: Shogun's Amiga border ended at native row 336 of
/// 400 — **that 336 was itself a defect, fixed in SQ-1029: the border is 400 rows
/// and `@split_window` was resetting window 0 to full width so the erase that
/// followed took its bottom four text rows out of window 7** — and Arthur's poles
/// at 379, and the raster composite showed those last
/// rows as an unpainted band inside the frame's own lower edge — measured on
/// `James Clavell's Shogun.adf` (release 295, serial 890321) as **64 native rows
/// of one flat colour** in both flanks, and on `Arthur - The Quest for
/// Excalibur.adf` (release 54, serial 890606) as **21**. Zork Zero's pillars
/// already reach row 400, which is why nobody saw it there.
///
/// **This is a composition change, not a scaling one, and that is the whole
/// point.** Raster builds the frame once at the 640x400 native screen
/// (`INFOCOM_V6_STD_WINDOW` doubled — SQ-0479) and hands the finished canvas to a
/// single resize, the way Bocfel's `flush_bitmap` stretch-blits its pixmap once;
/// so the flanks must be complete BEFORE that scale, and no band may be scaled on
/// its own. Doing it here rather than at draw time is what keeps the corners
/// agreeing for free — the property SQ-0698's suite asserts as a RELATION between
/// bands on the hybrid side, and which raster gets structurally.
///
/// The flank columns are the ones either side of the story window's declared
/// native rect, which is where the hybrid ring's side bands map back to. The
/// classification reads `gfx`, the bare graphics canvas — "opaque" is not
/// "artwork" (SQ-0500) — while the pixels are cut from `canvas`, which already
/// carries the painted ground and the window pages, exactly as
/// [`flank_tiled_source`] pairs them. Everything above the art's own extent comes
/// back byte-for-byte, so a flank with nothing to extend is untouched.
///
/// **A game with a command menu under its story window is excluded** — the same
/// exclusion the hybrid ring makes when it builds `tiled_flanks`, and SQ-0819 is
/// what it costs to omit it. Journey's left column is a PICTURE seated in a
/// panel, not a border to extend: measured on `Journey - The Quest Begins.adf`
/// (release 30, serial 890322) its illustration paints native rows 25..279 in
/// columns 0..264, its picture window is `(8,16) 248x272`, and the menu strip's
/// rule and "The Party" label sit at rows 288 and 296. Unrecognised as any of
/// the three border titles, that column fell through [`v6_border::recognize`] to
/// `ArthurPoles`, which cut a 4-row strip at 90% of 279 and tiled it — canyon
/// wall and all — down to native row 400, straight over the menu strip. The
/// player saw "Individual Commands" alone with the art column smeared across
/// where "The Party" belongs. Release 83 (`journey-r83-s890706.z6`, story window
/// bottom 304) has the identical shape, so this was never medium-specific.
///
/// **SQ-1032 gave it the FRAME rather than the native screen**, and the two are the
/// same thing until the composite extends: the rows copied verbatim off `canvas` are
/// still the game's own screen (`frame.native.1`), and only the number of rows the
/// band is asked to FILL changes (`frame.canvas_h`). `flank_source` has always taken
/// those as separate arguments — "extend only when the pane is taller than the art" is
/// Bocfel's own guard and it reads a desired height, not a screen — so an extended
/// frame tiles further down the same recipe rather than a new one.
fn extend_raster_flanks(
    canvas: &mut image::RgbaImage,
    gfx: &image::RgbaImage,
    story: Option<&crate::engine::PositionedWindow>,
    chrome: &[&crate::engine::PositionedWindow],
    frame: crate::render::v6_layout::RasterFrame,
    cell: zvm::screen::V6Cell,
) {
    let native = frame.native;
    let Some(story) = story else { return };
    if menu_strip_below_story(story, gfx, chrome, native, cell) {
        return;
    }
    let native_h = native.1 as u32;
    let right = (story.x_px as u32).saturating_add(story.w_px as u32);
    for (x0, x1) in [(0, (story.x_px as u32).min(native.0 as u32)), (right.min(native.0 as u32), native.0 as u32)] {
        if x1 <= x0 {
            continue;
        }
        let art = crate::render::v6_border::art_extent(gfx, x0, x1);
        let Some(src) = crate::render::v6_border::flank_source(
            canvas,
            gfx,
            x0,
            x1,
            art,
            native_h,
            0,
            frame.canvas_h,
        ) else {
            continue;
        };
        for y in 0..src.height().min(canvas.height()) {
            for x in 0..src.width().min(canvas.width().saturating_sub(x0)) {
                canvas.put_pixel(x0 + x, y, *src.get_pixel(x, y));
            }
        }
    }
}

/// A native `(x, y, w, h)` crop of the chrome canvas, as a band draw takes it.
type BandCrop = (u32, u32, u32, u32);

/// How a flank's border column reaches the screen (SQ-0750).
///
/// In hybrid, never rasterise what the game printed as a character: a border made
/// of the game's own characters is stamped as those characters, and only pixels the
/// paint runs cannot account for — genuine artwork — are carried as a bitmap.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BorderInk {
    /// Artwork: a one-native-row crop of the chrome canvas, replicated down the band.
    Band(BandCrop),
    /// The game's own character, with its Z-machine style bits and packed colours.
    ///
    /// SQ-0779: `col` is the one column the character is STAMPED in and `native` is
    /// the `[x0, x1)` of its own 8-pixel text cell — which is wider than one terminal
    /// column wherever the letterbox scale exceeds one column per native cell (2.93
    /// at the 236x68 terminal this was reported from). The extension's rect covers
    /// the whole cell so the artwork beside it stops at a native cell boundary and
    /// the cell's own ground is drawn rather than left to the backdrop; the glyph
    /// still stands in exactly ONE of those columns, which is what keeps SQ-0750's
    /// doubled rule from coming back.
    Glyph { ch: char, style: u8, fg: u32, bg: u32, col: u16, native: (u32, u32) },
}

/// One of a flank's border columns carried down the reclaimed gap: where it is
/// drawn, and what it is drawn WITH.
type FlankBorderExt = (Rect, BorderInk);

/// What [`menu_flank_panel`] resolves for a side flank: the panel background, the
/// rect to flood with it, the destination rect for the vertically centred art,
/// and the native `(x, y, w, h)` crop of the canvas to draw into it.
type FlankPanel = (image::Rgba<u8>, Rect, Rect, BandCrop);

/// SQ-0828: the whole-cell box closest in ASPECT to a `dw × dh` device rect.
///
/// A band is placed at cell granularity, so the art is drawn into `cols · cw` by
/// `rows · ch` device pixels — and [`menu_flank_panel`] used to reach those by ceiling
/// each axis on its own. A cell is 8 wide and 18 tall, so the two ceilings round by
/// wildly different amounts: Journey's 222x254 plate at a 80x24 pane (uniform scale
/// exactly 1.0) came out 224x270 — x1.0090 against y1.0630, a **5.3% aspect error**,
/// with the picture stretched vertically for no reason anyone chose. Cell quantization
/// on a coarse grid is unavoidable; picking the WORST corner of it is not.
///
/// So both axes are chosen together, against the exact criterion
/// `cols · cw · dh == rows · ch · dw` — the cross-product, so no division and no
/// tolerance — over the four boxes the ideal falls between (each axis floored and
/// ceiled), and the least wrong one wins. Ties go to the larger box, because two boxes
/// with the same aspect error are the same picture and the bigger one is more of it.
/// Every candidate is within ONE cell of the ideal on each axis, so this can neither
/// grow the art to fill its column nor starve it — it only chooses which corner of the
/// grid to land on. On the same 80x24 pane the answer becomes 224x252: a 1.7% error, and
/// the floor for that pane, since 14 rows of 18px cannot express 254/222 any better.
///
/// `max_cols`/`max_rows` are the caller's own bounds and are applied to every candidate,
/// so a capped axis still gets the best partner for the width it is left with.
fn aspect_cells(dw: f32, dh: f32, cw: u32, ch: u32, max_cols: u16, max_rows: u16) -> (u16, u16) {
    let (cf, cc) = ((dw / cw as f32).floor() as i32, (dw / cw as f32).ceil() as i32);
    let (rf, rc) = ((dh / ch as f32).floor() as i32, (dh / ch as f32).ceil() as i32);
    let mut best: Option<(u16, u16, f64, u32)> = None;
    for c in [cf, cc] {
        for r in [rf, rc] {
            let cols = (c.max(1) as u16).min(max_cols.max(1));
            let rows = (r.max(1) as u16).min(max_rows.max(1));
            // Normalised so this is a RATIO error and comparable across sizes.
            let err = ((cols as f64 * cw as f64 * dh as f64) - (rows as f64 * ch as f64 * dw as f64))
                .abs()
                / (dw as f64 * dh as f64).max(1.0);
            let area = cols as u32 * rows as u32;
            if best.is_none_or(|(_, _, be, ba)| err < be - 1e-9 || (err < be + 1e-9 && area > ba)) {
                best = Some((cols, rows, err, area));
            }
        }
    }
    let (cols, rows, ..) = best.expect("the 2x2 candidate box is never empty");
    (cols, rows)
}

/// SQ-0547: treat a Menu-plan side flank as a PANEL rather than a top-anchored
/// strip of art over bare backdrop.
///
/// Journey's left column holds an illustration far shorter than the column is at
/// a tall pane, so the reclaimed space below it showed the theme backdrop and the
/// column stopped reading as part of the game. Returns
/// `(panel background, fill rect, destination rect for the art, native crop)`:
///
///   * the background is the game's OWN panel colour, sampled from the outer edge
///     of the flank art — the colour Journey paints around its picture (rgb 34,34,34)
///     — so the filled column matches the art instead of the theme or the letterbox;
///   * the FILL rect is the panel's own extent, which is NOT the band's (SQ-0747
///     item A): it runs strictly BETWEEN the flank's two border columns, `inner`
///     (the rule against the story box) and `outer` (the frame's far edge), so
///     neither border stands on the panel's ground. See the comment on it below;
///   * the destination rect keeps the band's horizontal placement exactly and
///     centres the art VERTICALLY in the column, at the uniform scale (the art's
///     own aspect ratio is preserved — SQ-0511's fix must not regress);
///   * the crop is the art's opaque row span across the flank's native columns.
///
/// `None` when the flank carries no art at all: there is then nothing to centre
/// and no colour to sample, so the caller keeps today's behaviour.
///
/// Measured against the GRAPHICS-only canvas (`gfx`), never the full chrome
/// canvas: the latter has the game's text rasterized into it, so its first opaque
/// pixel in this column is a light text band and both the row span and the
/// sampled panel colour would come out wrong. Same canvas the strip decomposition
/// consults to answer "is there art behind this strip?".
fn menu_flank_panel(
    band: Rect,
    viewport: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
    gfx: &image::RgbaImage,
    inner: Option<Rect>,
    outer: Option<Rect>,
) -> Option<FlankPanel> {
    if band.width == 0 || band.height == 0 {
        return None;
    }
    // The flank's native column range: left of the story box, or right of it.
    let story_x0 = story.x_px as u32;
    let story_x1 = (story.x_px as u32 + story.w_px as u32).min(native.0 as u32);
    let (nx0, nx1) = if band.x < viewport.x {
        (0, story_x0.min(gfx.width()))
    } else {
        (story_x1.min(gfx.width()), (native.0 as u32).min(gfx.width()))
    };
    if nx1 <= nx0 {
        return None;
    }
    // The art's opaque BOUNDING BOX in that column, plus the first opaque pixel on
    // its top row — the art's outer edge, i.e. the panel colour the game painted.
    //
    // The box must be tight on BOTH axes. Journey's picture occupies native x 5..226
    // of a 240-wide column, so cropping the whole column would drag its transparent
    // side margins into the drawn image, and those render as dark cells ON TOP of the
    // panel fill — a strip of "missing background" down the right of the picture.
    let mut top: Option<(u32, image::Rgba<u8>)> = None;
    let (mut ax0, mut ax1, mut ay1) = (u32::MAX, 0u32, 0u32);
    for y in 0..gfx.height() {
        let mut row_first: Option<u32> = None;
        for x in nx0..nx1 {
            if gfx.get_pixel(x, y)[3] >= 128 {
                if row_first.is_none() {
                    row_first = Some(x);
                }
                ax0 = ax0.min(x);
                ax1 = ax1.max(x);
            }
        }
        if let Some(x) = row_first {
            if top.is_none() {
                top = Some((y, *gfx.get_pixel(x, y)));
            }
            ay1 = y;
        }
    }
    let (ay0, panel) = top?;
    if ax1 < ax0 {
        return None;
    }
    let (art_w, art_h) = (ax1 - ax0 + 1, ay1 - ay0 + 1);
    let (cw, ch) = (cell_px.0.max(1) as u32, cell_px.1.max(1) as u32);
    // Horizontal placement is unchanged from the band mapping: the art's native left
    // edge through the same scale. Only the VERTICAL anchor moves (centred).
    let x = band.x + ((scale.off_x as f32 + ax0 as f32 * scale.s) / cw as f32).floor() as u16;
    // The art's ideal device extent at the uniform scale — what the cell box below is
    // trying to be.
    let (dw, dh) = (art_w as f32 * scale.s, art_h as f32 * scale.s);
    let mut col_cap = band.right().saturating_sub(x).max(1);
    // Keep one column of panel fill between the picture and the divider, so the
    // panel frames the art on that side the way the art's own native left margin
    // frames it on the other. Only applies when the divider lies to the RIGHT of the
    // art — i.e. a left-hand flank, the only kind any Menu-plan game has; a right-hand
    // flank keeps today's placement. It is a CAP on the columns, and the rows follow
    // from the aspect below, so narrowing the picture can no longer squash it.
    if let Some(dx) = inner.map(|d| d.x).filter(|&dx| dx > x) {
        let limit = dx.saturating_sub(x).saturating_sub(1);
        if limit > 0 {
            col_cap = col_cap.min(limit);
        }
    }
    let (cols, rows) = aspect_cells(dw, dh, cw, ch, col_cap, band.height);
    let y = band.y + (band.height - rows) / 2;
    // SQ-0747 item (A): the FILL is the panel's own extent, and the band is wider than
    // that. A band runs to the story VIEWPORT's edge, and the viewport is quantized
    // INWARD to whole cells (`story_viewport_box` ceils its left edge), while the
    // frame's inner rule is quantized OUTWARD to the cells its ink covers. Between the
    // two there can be a leftover column belonging to neither — one, at the user's
    // 159- and 163-column panes; none at 138, which is why this came and went with the
    // pane. Flooding the whole band put the picture column's ground into that column,
    // i.e. the panel painted past the rule and up against the story text. Stop the
    // flood at the rule and the column falls back to the story's own ground, which is
    // what stands beside it.
    //
    // Under the IBM PC profile this changes nothing anywhere measured: that frame's
    // rule is a reverse-video SPACE, which inks its whole 8-pixel text cell, so the
    // rule's cells already reach the viewport and there is no leftover column. The
    // discriminator is the geometry, not the profile.
    //
    // SQ-0747, second pass: and it stops SHORT of the rule, not level with it. The
    // bound was inclusive on both ends — the fill began at the band's own left edge,
    // which is the OUTER border's column, and ran through the inner rule's last
    // column — so the panel's ground was painted into the two cells the frame's side
    // borders stand in. Those borders reach the screen as an image whose crop is the
    // whole text cell (SQ-0750), and a box glyph's padding is transparent, so the
    // panel colour showed through around the stroke: the user's *"the amiga build
    // border lines around the art have the artwork's background color … it is the
    // fill color that matches the artwork"*. A border is not part of the panel. Left
    // out of the fill, its cell keeps the ring's own never-painted ground — the same
    // ground the frame's top and bottom rules stand on — and the stroke reads as one
    // line with them.
    let (lo, hi) = if band.x < viewport.x {
        // Left flank: outer border at the pane edge, inner rule against the story.
        (outer.map_or(band.x, |o| o.right()), inner.map_or(band.right(), |d| d.x))
    } else {
        // Right flank: the inner rule is its LEFT edge, the outer border its right.
        (inner.map_or(band.x, |d| d.right()), outer.map_or(band.right(), |o| o.x))
    };
    let lo = lo.clamp(band.x, band.right());
    let hi = hi.clamp(lo, band.right());
    let fill = Rect::new(lo, band.y, hi - lo, band.height);
    Some((panel, fill, Rect::new(x, y, cols, rows), (ax0, ay0, art_w, art_h)))
}

/// Which of a flank's two border columns [`flank_border_extension`] is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FlankBorder {
    /// The rule dividing the flank from the story box.
    Inner,
    /// The frame's OUTER edge — the far side of the flank, against the pane margin.
    Outer,
}

/// SQ-0511 fix (Journey Menu plan): a border-column extension for one side flank. The
/// flank picture is drawn at the UNIFORM scale (aspect preserved), so a gap opens
/// between the flank art's uniform-scaled bottom (the story's native bottom) and the
/// bottom-anchored menu. This returns a NARROW band spanning that gap over one of the
/// flank's full-height border columns, plus a 1-native-row crop of those columns to
/// replicate down it. The column is uniform, so the vertical replicate is invisible;
/// the rest of the gap is left undrawn (transparent → theme backdrop, matching the
/// flank's own never-painted background beside the divider).
///
/// `which` picks the column: [`FlankBorder::Inner`] is the rule abutting the story box,
/// [`FlankBorder::Outer`] the frame's far edge (SQ-0758). Both are located by the same
/// probe from their own side, so one calculation bounds the panel and draws both of its
/// borders. Returns `None` when that column is not there or the gap is empty.
/// `menu_top_row` is the bottom-anchored menu strip's top cell (viewport bottom).
///
/// SQ-0750: …unless the game printed that border as a CHARACTER, in which case the
/// column comes back as [`BorderInk::Glyph`] and is stamped, not uploaded. The test
/// is content, not position and not the interpreter profile: the graphics-only
/// canvas must carry no artwork in the border's own native columns over the story's
/// rows, AND one of the game's own paint runs must ink them. Both hold for Journey's
/// four frame rules under either profile — box-drawing glyphs on the Amiga, reverse-
/// video spaces on the IBM PC — and neither holds for Zork Zero's, Shogun's or
/// Arthur's side columns, which are pictures and stay bitmaps.
///
/// The glyph column is placed by [`run_cell`], the very mapping the frame's top rule
/// uses for its own corner, so the rule cannot land a column off the `┌` it hangs
/// from — and it is one column wide by construction, where the band's cell span can
/// be two and would have stamped a double rule.
///
/// SQ-0883: `ink` is the chrome canvas as the game PAINTED it — art, glyph ink and
/// painted ground — frozen before any window's page floods the rest. It is not the
/// canvas the band is cut from, and the difference is the whole of this quest. The
/// probe grows a contiguous opaque run outward from the story's edge, and a page
/// fill is opaque everywhere: on `stories/Journey.po` (the ProDOS press, release 77
/// serial 890616, booted as `AppleIIgs`) the AppleIIgs profile presents its picture
/// window on a page, so the run that should have stopped on the frame's rule at
/// native x 72..80 ran the full width of the flank, 0..80. Measured at a 169x47
/// pane: an 83x1 crop THROUGH the illustration, replicated down 738 device pixels
/// over the whole left column — the artwork replaced by a smear of one of its own
/// rows. `honor_game_colours = false` skips the flood and was correct throughout,
/// which is why one mode saw it and the other did not. Classify from what was
/// painted, cut from what is presented: the same pairing [`flank_tiled_source`]
/// makes between `gfx` and the chrome canvas.
#[allow(clippy::too_many_arguments)]
fn flank_border_extension(
    band: Rect,
    pane: Rect,
    viewport: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
    ink: &image::RgbaImage,
    gfx: &image::RgbaImage,
    runs: &[&crate::engine::PxText],
    menu_top_row: u16,
    which: FlankBorder,
    cell: zvm::screen::V6Cell,
) -> Option<FlankBorderExt> {
    let cw = cell_px.0.max(1) as f32;
    let s = if scale.s <= 0.0 { 1.0 } else { scale.s };
    let sy0 = story.y_px as u32;
    let sy1 = (story.y_px as u32 + story.h_px as u32).min(ink.height());
    if sy1 <= sy0 {
        return None;
    }
    // A mid-story native row: the border column is inked here regardless of where the
    // picture (which may not span the full column height) begins or ends.
    let mid = sy0 + (sy1 - sy0) / 2;
    let opaque = |x: u32, y: u32| x < ink.width() && y < ink.height() && ink.get_pixel(x, y)[3] >= 128;
    // The divider/border is the contiguous opaque native column run abutting the story
    // box edge on this flank (left flank → run ending at the story's left edge; right
    // flank → run starting at the story's right edge), sampled at that mid-story row.
    let story_x0 = story.x_px as u32;
    let story_x1 = (story.x_px as u32 + story.w_px as u32).min(native.0 as u32);
    // SQ-0742: a border is not always a filled block. Journey under the Amiga profile
    // draws its frame with box-drawing GLYPHS, and a `│`'s ink sits in the middle of
    // its 8-pixel text cell — so the native column immediately abutting the story box
    // is that cell's own blank padding. Probing that one column found nothing and the
    // whole extension was abandoned, which is why the Amiga frame produced NO divider
    // bands at all while the IBM PC profile produced one per flank. The side borders
    // then stopped where the game's canvas ends (native row 400 — terminal row 39 of a
    // 68-row pane) and the frame was simply absent for the eighteen rows between there
    // and the menu: the user's "big chunk of the border missing from the right side (at
    // the bottom)". Look across ONE text cell for the ink before giving up. A
    // reverse-video block border inks offset 0, so it takes exactly the path it did.
    let font_w = u32::from(cell.w());
    let seek_ink = |from: u32, outward_is_left: bool| -> Option<u32> {
        (0..font_w).find_map(|d| {
            let x = if outward_is_left { from.checked_sub(d + 1)? } else { from + d };
            opaque(x, mid).then_some(x)
        })
    };
    // The run grows from wherever the ink was found, so one arm serves both edges:
    // `from` is the native column the probe starts at, `grows_right` says which way it
    // runs from there, and `(lo, hi)` are the FLANK's own native columns — a run must
    // never leave them. Without that bound the outward probe on a right-hand flank
    // walks left out of the flank, straight through the story window's own opaque page,
    // and reports a "border" four hundred pixels wide.
    let run_from = |from: u32, grows_right: bool, lo: u32, hi: u32| -> Option<(u32, u32)> {
        if grows_right {
            let left = seek_ink(from, false).filter(|x| (lo..hi).contains(x))?;
            let mut x = left;
            while x < hi && opaque(x, mid) {
                x += 1;
            }
            Some((left, x))
        } else {
            let mut x = seek_ink(from, true).filter(|x| (lo..hi).contains(x))?;
            let right = x + 1;
            while x > lo && opaque(x - 1, mid) {
                x -= 1;
            }
            Some((x, right))
        }
    };
    let left_flank = band.x < viewport.x;
    // The flank's own native columns: left of the story box, or right of it.
    let (lo, hi) = if left_flank { (0, story_x0) } else { (story_x1, native.0 as u32) };
    if hi <= lo {
        return None;
    }
    let (dnx0, dnx1) = match (which, left_flank) {
        // The rule abutting the story box: probe from the story edge, outward.
        (FlankBorder::Inner, true) => run_from(story_x0, false, lo, hi)?,
        (FlankBorder::Inner, false) => run_from(story_x1, true, lo, hi)?,
        // SQ-0758: the frame's OUTER edge — the far side of the flank. Probed from
        // the screen edge inward, one text cell's worth, exactly as the inner rule is
        // probed from the story edge outward. It was never located at all, so under a
        // Menu plan the flank's outer border simply did not exist between the `┌` on
        // the top rule and the `└` on the bottom one: `menu_flank_panel` floods the
        // column and draws only the picture's bounding box, and that box does not
        // reach the border.
        (FlankBorder::Outer, true) => run_from(lo, true, lo, hi)?,
        (FlankBorder::Outer, false) => run_from(hi, false, lo, hi)?,
    };
    if dnx1 <= dnx0 {
        return None;
    }
    // …and an outer run that is ARTWORK is not a border. Under the IBM PC profile
    // Journey's picture starts at native x 5 with no border outside it, so the outward
    // probe finds the illustration itself and would carry a one-pixel slice of it down
    // the whole column. The chrome canvas cannot tell the two apart — the picture is
    // rasterized into it — so ask the graphics-only canvas, the same discriminator
    // SQ-0750 settled on: a band is art only when it is actually artwork.
    if which == FlankBorder::Outer && (dnx0..dnx1).any(|x| x < gfx.width() && mid < gfx.height() && gfx.get_pixel(x, mid)[3] >= 128) {
        return None;
    }
    // Device cell x-range covering the divider columns (through the uniform scale), and
    // the device row where the flank's uniform-scaled art bottoms out (the story's
    // native bottom). The extension spans from there down to the menu strip top.
    let dcell0 = pane.x + ((scale.off_x as f32 + dnx0 as f32 * s) / cw).floor() as u16;
    let dcell1 = pane.x + ((scale.off_x as f32 + dnx1 as f32 * s) / cw).ceil() as u16;
    if dcell1 <= dcell0 || menu_top_row <= band.y {
        return None;
    }
    // The divider runs the WHOLE flank column, from its top down to the menu strip.
    // It used to start where the flank art bottomed out, which was fine while the art
    // was top-anchored and carried its own divider pixels above that row. Now the art
    // is centred and cropped to its own bounding box (SQ-0547), so those pixels are no
    // longer drawn — and the divider column lies to the RIGHT of the picture, so
    // running it full height covers the gap without touching the art.
    let ext = Rect::new(dcell0, band.y, dcell1 - dcell0, menu_top_row - band.y);
    // SQ-0750, THE CONTENT TEST: do the game's own paint runs account for this
    // column's pixels? Two conditions, and both are about what is in the column
    // rather than where it sits or which interpreter profile is loaded:
    //
    //   1. the GRAPHICS-only canvas is clear across the border's native columns for
    //      the story's whole row span — no artwork is hiding under the ink; and
    //   2. one of the game's paint runs covers those columns at the row we sampled,
    //      and the character it puts there actually inks something (a bare space
    //      that is not reverse-video inks nothing, so it cannot be what we found).
    //
    // Then the column is a CHARACTER, and hybrid draws characters as characters.
    // The rule reaches the screen through `run_cell` — the same mapping the frame's
    // top rule uses to place its corner — so the two meet in one column instead of
    // a font glyph sitting above a resampled bitmap of the same character.
    let text_border = (|| {
        if (dnx0..dnx1).any(|x| {
            (sy0..sy1).any(|y| x < gfx.width() && y < gfx.height() && gfx.get_pixel(x, y)[3] >= 128)
        }) {
            return None;
        }
        let t = runs.iter().copied().find(|t| {
            let px0 = t.x.max(1) as u32 - 1;
            let w = t.text.chars().count().max(1) as u32 * font_w;
            cell.rows_px(t.y).contains(&mid) && (px0..px0 + w).contains(&dnx0)
        })?;
        let idx = (dnx0 - (t.x.max(1) as u32 - 1)) / font_w;
        let ch = t.text.chars().nth(idx as usize)?;
        if ch == ' ' && t.style & 1 == 0 {
            return None;
        }
        let gnx0 = (t.x.max(1) as u32 - 1) + idx * font_w;
        // SQ-0783: …aligned outward when it is the SCREEN's own edge cell, so the
        // frame reaches the pane instead of leaving the column beside it blank.
        let col = edge_glyph_col(gnx0, native.0 as u32, scale, cell_px, pane, cell)
            .unwrap_or_else(|| run_cell(t, scale, cell_px, pane, cell).0 + idx as i32);
        let col = u16::try_from(col).ok()?;
        if col < band.x || col >= band.right() {
            return None;
        }
        // SQ-0779: the character's own native text CELL, and EVERY terminal column any
        // part of it falls in — floor on the near edge, ceil on the far one. The glyph
        // stands in one of them (`col`, exactly as before); the rest are the cell's own
        // blank padding, and they belong to the border rather than to the picture beside
        // it, which is the whole of the user's ruling.
        //
        // The full overlap set, not the rounded one, because this rect is what the
        // artwork's span is trimmed against and a band's source crop is its DESTINATION
        // mapped back through the same scale. Round the far edge and the column left
        // over is one whose device span still reaches into the cell — so the crop starts
        // a native pixel or two inside it, and at a large enough scale that is where the
        // stroke lives. Widened, never narrowed: the cell's pixels are the border's.
        let gnx1 = gnx0 + u32::from(cell.w());
        let dev = |nx: u32| (scale.off_x as f32 + nx as f32 * s) / cw;
        let x0 = (pane.x as i32 + dev(gnx0).floor() as i32).clamp(band.x as i32, col as i32) as u16;
        let x1 = (pane.x as i32 + dev(gnx1).ceil() as i32).clamp(col as i32 + 1, band.right() as i32) as u16;
        Some((
            Rect::new(x0, ext.y, x1 - x0, ext.height),
            BorderInk::Glyph { ch, style: t.style, fg: t.fg, bg: t.bg, col, native: (gnx0, gnx1) },
        ))
    })();
    if let Some(g) = text_border {
        return Some(g);
    }
    // SQ-0750: the crop is the native columns those CELLS cover, not the inked run
    // alone. `draw_chrome_band_stretched` resizes the crop to fill the band, so the
    // crop's width IS the horizontal magnification — and cropping to the ink alone
    // magnifies by (cell span / ink width) instead of by the uniform letterbox scale.
    // For a border the game drew as a reverse-video SPACE that is the same number
    // (the run inks its whole 8px text cell) and nothing moves. For one drawn with a
    // box-drawing GLYPH it is not: a `│`'s stroke is ONE pixel inside its 8-pixel
    // cell, a sixteenfold blow-up at the pane this was measured at, so Journey's
    // Amiga frame had its two vertical borders inflated into solid filled bars two
    // and one terminal columns wide for their whole height — the IBM PC profile's
    // reverse-video idiom, standing in the same line as the box glyphs the top,
    // bottom and menu rows draw as crisp text. The user's *"we are mixing the reverse
    // space into the amiga line drawing"*. Taking the whole cell keeps the glyph's own
    // blank padding, so the stroke comes out at the width the game drew it and the
    // padding stays transparent over the panel behind. Widened, never narrowed: the
    // ink itself must survive whatever way the two roundings fall.
    let inv_x =
        |cell: u16| ((((cell.saturating_sub(pane.x)) as f32 * cw) - scale.off_x as f32) / s).round().max(0.0) as u32;
    let cnx0 = inv_x(dcell0).min(dnx0);
    let cnx1 = inv_x(dcell1).max(dnx1).min(ink.width());
    if cnx1 <= cnx0 {
        return None;
    }
    // SQ-0883: and it must be a RULE. This band replicates ONE native row down the
    // whole flank — the ring's single licensed anisotropy — and that is only
    // invisible because a rule is uniform down its length. A crop wider than a rule
    // is a slice of something else, and stretching it paints that slice over the
    // column in bands.
    //
    // The probe above reads ink rather than window pages (the first half of this
    // quest), but ink is not only the frame: `blit_paint_ground` puts the game's own
    // PAINTED runs into the same layer, deliberately (SQ-0706), and a game that
    // paints a ground across the flank hands the run the same uninterrupted walk a
    // page did. Journey's ProDOS press (release 77, its menu frame) is that shape —
    // window 1 is a full-screen grid carrying 65 paint runs — and the run came back
    // **252 native px**, 31 text cells, cropped at row 144 straight through the
    // illustration and stretched to 464x972 device px over the entire left half of
    // the pane. Measured at the user's own 129x60 pane; it is in their
    // `/dump-windows` and in `ring_scout --story stories/Journey.po --keys n --taps 2`.
    //
    // So the licence is granted on the crop's own width instead of on what the probe
    // walked through, which no future flooding of that layer can widen: one native
    // text cell, plus the terminal column each end's outward rounding above can add.
    // Every real border in the corpus is a reverse-video space or a box glyph, and
    // both live inside one cell by construction.
    let widest = font_w + 2 * (cw / s).ceil() as u32;
    if cnx1 - cnx0 > widest {
        return None;
    }
    Some((ext, BorderInk::Band((cnx0, mid, cnx1 - cnx0, 1))))
}

/// Map a chrome run's native top-left game pixel to its pane-absolute terminal
/// cell (col, row) through the letterbox `scale` — the same mapping the pixel
/// ring and the inside-story overlay use, so a text strip lines up exactly with
/// the art strips beside it.
/// A v6 window's native pixel rect as terminal CELLS, both corners mapped exactly as
/// [`run_cell`] maps a run's origin: through the scale, then ROUNDED. Rounding (not
/// ceil) on the far edge is what keeps a 20px status strip — 1.25 cells — from
/// claiming a second row and eating the first line of story under it. (SQ-0584)
///
/// `row_shift` slides the whole native screen by whole terminal ROWS, for the cell
/// path's packing (SQ-0697): there the native screen is anchored on the first inked
/// chrome row rather than on the pane's top edge, and a window's erased ground has
/// to move with the glyphs painted on it. Zero everywhere else.
fn px_rect_to_cells(
    pw: &PositionedWindow,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    row_shift: i32,
) -> Rect {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    let to_col = |px: f32| pane.x as f32 + (scale.off_x as f32 + px * scale.s) / cw;
    let to_row = |py: f32| pane.y as f32 + (scale.off_y as f32 + py * scale.s) / ch + row_shift as f32;
    let x0 = to_col(pw.x_px as f32).round().max(pane.x as f32) as u16;
    let y0 = to_row(pw.y_px as f32).round().max(pane.y as f32) as u16;
    let x1 = to_col(pw.x_px.saturating_add(pw.w_px) as f32).round().max(x0 as f32).min(pane.right() as f32) as u16;
    let y1 = to_row(pw.y_px.saturating_add(pw.h_px) as f32).round().max(y0 as f32).min(pane.bottom() as f32) as u16;
    Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

/// SQ-0892: the X-axis counterpart of SQ-0543's row packing, for a text strip that
/// has no horizontal structure to preserve. `Some(origin)` gives the terminal column
/// the strip's LEFTMOST native text cell occupies, after which every run is placed by
/// its offset in native cells and each character advances one column.
///
/// **Why a strip needs its own map at all.** A run occupies two widths at once. Its
/// NATIVE footprint is `chars × 8` game pixels, which the letterbox scale stretches
/// to `chars × 8 × s` device pixels — what the composite draws, and what the art
/// around it is drawn at. Its RENDERED width is `chars` terminal cells, because a
/// glyph is one cell and no scale changes that. [`run_cell`] maps the run's first
/// native pixel and lets the characters advance from there, which piles the whole
/// difference onto the right-hand end — and the difference grows with the run's
/// LENGTH, so two runs of different lengths that the game aligned with each other no
/// longer are. MEASURED on Shogun's credits at a 100x40 pane (s = 1.225): the game
/// centres all nine lines on native x=320, and per-run mapping lands their centres in
/// columns 43, 44, 44, 45, 45.5, 46, 47, 47 and 48 — a five-column wobble, on text
/// whose entire design is that it is centred. Indexing by native cell instead brings
/// all nine back to column 49, which is exactly where native x=320 maps.
///
/// This is SQ-0543's argument on the other axis, and that comment already makes it:
/// *"Inside a TEXT strip there is no art to stay aligned with — having no frame
/// graphics behind it is what MAKES it a text strip — so the game's own row structure
/// is the truth to preserve, not the device-pixel position."* Column structure is the
/// truth on the same grounds. Only the Y half was ever written.
///
/// **Why it is refused when a row carries more than one run, which is the whole of
/// the guard.** Native-cell indexing has slope 1 column per native cell where the
/// device map has slope `s`, so the two diverge linearly with distance from the
/// origin. Over a block the game composed in its own text grid that is the point.
/// Over FIELDS the game pinned to SCREEN positions it is a disaster, and the corpus
/// says so — each of these was measured, not supposed:
///
/// * Journey's Amiga menu band puts verbs and `▌` dividers on one row across the full
///   640px; moving them by native offset drags every column in from the pane edges
///   and `journey_amiga_menu_dividers_line_up_down_the_panel` fails with the divider
///   8 columns adrift of the text it belongs to.
/// * Shogun's own gameplay status band is `Erasmus` … `SHOGUN` … `Score:` `0` on one
///   row, right-justified at native x=586; native indexing moves the left field 8
///   columns right and the value 5 left, so the band's contents shrink away from the
///   flood that is still drawn edge to edge.
/// * Arthur's status row and Zork Zero's location/score row are the same shape.
///
/// A row carrying one run has no such relationship to break: nothing stands beside it,
/// and its only alignments are with the OTHER ROWS, which is precisely what this
/// preserves and what per-run mapping destroys. On the corpus the two classes separate
/// cleanly — Shogun's credits are 9 rows of one run; Journey's band, Arthur's and Zork
/// Zero's status rows and advent's bar all carry two or more after merging.
///
/// **The origin.** The strip's leftmost native pixel through the device map, plus half
/// the strip's SLACK — the columns by which the whole block's native footprint exceeds
/// the glyphs that will be drawn in it. Distributing the slack evenly is what keeps a
/// centred block centred where the composite centres it (Shogun's, exactly). Slack is
/// floored at zero: below one column per native cell the glyphs are WIDER than their
/// footprint and there is nothing to distribute, so the block keeps the left edge the
/// game gave it rather than being pushed off the pane. That case is live — Shogun at a
/// 74-column pane resolves to s = 0.925.
fn strip_native_origin(
    runs: &[&crate::engine::PxText],
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    cell: zvm::screen::V6Cell,
) -> Option<i32> {
    // The v6 text cell is 8x16 (SQ-0479).
    let font_w = i32::from(cell.w());
    if runs.is_empty() {
        return None;
    }
    // One run per native text row, or the row has fields whose screen positions are
    // the truth. Counted on the runs as they will be DRAWN, so the caller must merge
    // first — Arthur emits his status a glyph at a time and would otherwise look like
    // 72 fields on one row.
    let mut per_row: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
    for t in runs {
        *per_row.entry(cell.row_of(t.y)).or_default() += 1;
    }
    if per_row.values().any(|&n| n > 1) {
        return None;
    }
    let x0 = runs.iter().map(|t| t.x.max(1) as i32 - 1).min()?;
    let x1 = runs.iter().map(|t| (t.x.max(1) as i32 - 1) + t.text.chars().count() as i32 * font_w).max()?;
    let cw = cell_px.0.max(1) as f32;
    let cells = ((x1 - x0) as f32 / font_w as f32).max(1.0);
    let footprint = (x1 - x0) as f32 * scale.s / cw;
    let slack = (footprint - cells).max(0.0);
    let left = pane.x as f32 + (scale.off_x as f32 + x0 as f32 * scale.s) / cw;
    Some((left + slack / 2.0).round() as i32)
}

/// Where the ring places a text run, in terminal cells.
///
/// # It maps the run's CELL through the scale, never its pixel
///
/// A run is positioned once here and then advances ONE TERMINAL COLUMN per
/// character. Those two rates agree only if the position is itself a whole number
/// of character cells — and on a proportional machine a run's pixel origin is not
/// (SQ-1009). Arthur's Amiga press advances ~10.4 native pixels per glyph against
/// a declared 8, so consecutive word-sized runs are placed ~30% further apart than
/// their own characters reach: the words stay correct and the GAPS between them
/// open up, by more the larger the scale. That is the shape the report described —
/// "more spaces are added between some words" — and why it moved with the pane
/// while the wrap point, which is engine-side, did not.
///
/// So the run's own grid cell is what goes through the scale. For a fixed pen
/// `gcol * cell.w` IS `t.x - 1` and this is the arithmetic it always was.
fn run_cell(
    t: &crate::engine::PxText,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    v6: zvm::screen::V6Cell,
) -> (i32, i32) {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    // The COLUMN comes from the engine's grid pen and the SUB-CELL OFFSET from the
    // run's own pixel — because the two answer different questions and only one of
    // them drifts (SQ-1009, SQ-1048).
    //
    // `gcol * cell.w` alone is the column's left edge, which is not where the game
    // painted: Arthur opens its status window at x=29 and sets the bar's first glyph
    // at x=35, two pixels into column 4. Flooring that to 32 walks the whole ribbon
    // left, and at a pane width of 93 the `C` of `Churchyard` crosses into the
    // flank's five columns of pole art and is overwritten by it.
    //
    // `t.x - 1` alone — what this did before the grid pen existed — is exact on any
    // machine whose pen IS the cell, and on Arthur's Amiga press climbs about 1.3
    // columns per glyph.
    //
    // So take the column from the grid and the remainder from the pen. On a fixed pen
    // `gcol == col_of(t.x)` and the two terms collapse back to `t.x - 1` exactly, so
    // every machine but the proportional one is bit-identical to the old answer.
    let sub_x = f32::from(t.x.max(1) - 1) - f32::from(v6.col_of(t.x)) * f32::from(v6.w());
    let sub_y = f32::from(t.y.max(1) - 1) - f32::from(v6.row_of(t.y)) * f32::from(v6.h());
    let px = f32::from(t.gcol) * f32::from(v6.w()) + sub_x;
    let py = f32::from(t.grow) * f32::from(v6.h()) + sub_y;
    let col = pane.x as i32 + ((scale.off_x as f32 + px * scale.s) / cw).round() as i32;
    let row = pane.y as i32 + ((scale.off_y as f32 + py * scale.s) / ch).round() as i32;
    (col, row)
}

/// Whether a row of painted runs is a reverse-video BAR — a band the game drew edge
/// to edge — as opposed to furniture built out of reversed spaces (SQ-1035).
///
/// # Why "all reversed" was not enough
///
/// A reverse-video SPACE is a solid block, and that is how these games draw a rule:
/// Arthur's F3 inventory paints its two column dividers as a single reversed space
/// per row, and Journey's IbmPc frame paints its side border the same way
/// (`journey_amiga_flank_border_is_a_stroke_not_a_filled_block` pins that the border
/// must stay a solid block, so the RUN is never in question). A row holding nothing
/// but those is entirely reversed and is not a bar, so flooding the cells around it
/// turned seven rows of Arthur's inventory white —
/// `machine-screenshots/amiga-arthur-inventory.png` shows a bare page with two thin
/// rules down it.
///
/// A bar has TEXT in it. Arthur's own status row, one window below, is all-reversed
/// AND carries `Churchyard`, and the same capture shows that row filled edge to edge
/// with dark letters on white — so the test is the presence of a non-blank run, and
/// both rows of that one frame come out right under it.
///
/// An earlier attempt gated on the runs reaching both window edges instead. That
/// reads the geometry rather than the content, and it broke Journey's border on the
/// IbmPc press; the rule that satisfies Arthur's inventory, Arthur's bar and
/// Journey's border together is what the runs CONTAIN.
pub(crate) fn row_is_reverse_bar<'a>(
    runs: impl IntoIterator<Item = &'a crate::engine::PxText>,
) -> bool {
    let mut any = false;
    let mut text = false;
    for t in runs {
        if t.style & 1 == 0 {
            return false;
        }
        any = true;
        text |= !t.text.trim().is_empty();
    }
    any && text
}

/// A glyph from the frame-drawing blocks: box drawing (U+2500..) and block elements
/// (U+2580..). What a game builds chrome geometry out of, and nothing any game's
/// prose contains. (SQ-0742's predicate, shared with SQ-0783.)
fn is_box_glyph(c: char) -> bool {
    // One predicate, in `bitfont` — where the sampler that has to keep these glyphs
    // meeting their neighbours also asks the question (SQ-1027). Two copies of a set
    // like this drift.
    crate::render::bitfont::must_tile(c)
}

/// SQ-0783: which column a LONE frame glyph belongs in when it stands at the game
/// SCREEN's own edge and its native 8-pixel text cell covers more than one.
///
/// A glyph is stamped in exactly one column — SQ-0750 chose that deliberately, since
/// repeating it across the cell's span draws a doubled rule — and [`run_cell`] rounds
/// the cell's LEFT edge, so the leftover columns fall to its right. Everywhere inside
/// the screen that is invisible and correct. At the screen's own right edge it is what
/// the user reported: *"starting at 159 width a blank space is added after"* the frame,
/// and the blank column 119 at 121x36. The frame's `┐`, its `┘` and the rule down that
/// side all stopped one column short of the pane while the game's screen ran to it —
/// at 157 pane columns the last native cell (632..640) spans 1.96 of them.
///
/// So an EDGE cell aligns outward: the first column its cell covers on the left, the
/// last on the right. Nothing else moves — an interior divider is a position of its
/// own (SQ-0742) and keeps [`run_cell`]'s answer — and where a terminal column is
/// about one native cell the span is one column and this returns that same column.
fn edge_glyph_col(
    nx0: u32,
    native_w: u32,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    cell: zvm::screen::V6Cell,
) -> Option<i32> {
    // The v6 text cell is 8x16 (SQ-0479).
    let font_w = u32::from(cell.w());
    let cw = cell_px.0.max(1) as f32;
    let dev = |nx: u32| (scale.off_x as f32 + nx as f32 * scale.s) / cw;
    if nx0 == 0 {
        Some(pane.x as i32 + dev(0).floor() as i32)
    } else if nx0 + font_w >= native_w {
        Some(pane.x as i32 + dev(native_w).ceil() as i32 - 1)
    } else {
        None
    }
}

/// The chrome windows an extended raster frame carries DOWN with its bottom edge,
/// as indices into `chrome` (SQ-1132).
///
/// A window qualifies when the game put it wholly below the story window and inside
/// the screen, and every run it carries is down there with it — which is what makes
/// it safe to move as a unit: its page, its cell grid, its runs and the rects the
/// prose spares all travel together, so nothing downstream can hold two opinions
/// about where it is.
///
/// `None` when some run below the story window belongs to a window that does NOT
/// qualify — one straddling the story's bottom edge, one reaching past the screen,
/// one with a size sentinel. That frame declines the extension exactly as it did
/// before this existed: an unrecognised shape is skipped, never guessed at.
///
/// Arthur is the frame it was written for. `arthur-r74-s890714.z6` publishes window
/// 3 across native (28, 384) 584x16 — the last text row of a 640x400 screen — and
/// prints one line into it whenever the parser fails, shrinking window 0 from
/// 584x192 to 584x176 to make room. Both windows are `Grid`s below the story; only
/// window 3 has runs below its bottom, so only window 3 moves.
fn bottom_anchored_chrome(
    chrome: &[&crate::engine::PositionedWindow],
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
) -> Option<Vec<usize>> {
    let story_bottom = i32::from(story.y_px) + i32::from(story.h_px);
    let mut out = Vec::new();
    for (i, w) in chrome.iter().enumerate() {
        let WinNode::Grid(g) = &w.node else { continue };
        // The run's own native TOP, spelled as `menu_band_runs` spells it, so the two
        // cannot disagree about which runs are the band.
        let top_of = |t: &crate::engine::PxText| i32::from(t.y.max(1)) - 1;
        let below = g.px_texts.iter().filter(|t| top_of(t) >= story_bottom).count();
        if below == 0 {
            continue;
        }
        // Every run of the window, and the window's own rect, on the far side of the
        // story window's bottom edge and inside the screen.
        let top = i32::from(w.y_px);
        if below != g.px_texts.len()
            || (w.h_px as i16) < 0
            || top < story_bottom
            || top + i32::from(w.h_px) > i32::from(native.1)
        {
            return None;
        }
        out.push(i);
    }
    Some(out)
}

/// One such window, re-seated `rows` native pixels lower.
///
/// Its runs carry SCREEN-absolute 1-based coordinates stamped at paint time, so they
/// move with the rect rather than following it; `grow` is the same row counted in
/// cells, and moves with them. The extension is a whole multiple of `cell.h` by
/// construction ([`crate::render::v6_layout::RasterFrame::extended`]), so the cell
/// counts stay whole.
fn bottom_anchor(
    w: &crate::engine::PositionedWindow,
    rows: u32,
    cell: zvm::screen::V6Cell,
) -> crate::engine::PositionedWindow {
    let px = u16::try_from(rows).unwrap_or(u16::MAX);
    let cells = u16::try_from(rows / u32::from(cell.h().max(1))).unwrap_or(u16::MAX);
    let mut out = w.clone();
    out.y_px = out.y_px.saturating_add(px);
    out.y = out.y.saturating_add(cells);
    if let WinNode::Grid(g) = &mut out.node {
        for t in &mut g.px_texts {
            t.y = t.y.saturating_add(px);
            t.grow = t.grow.saturating_add(cells);
        }
    }
    out
}

/// Every paint run the game put BELOW its story window — the content of the
/// bottom-anchored command strip, and the whole of it (SQ-0765).
///
/// Blank runs count: a reverse-video SPACE is how Journey draws the column dividers
/// down its menu, so a row of nothing but dividers is a menu row like any other.
fn menu_band_runs<'a>(
    runs: &[&'a crate::engine::PxText],
    story: &crate::engine::PositionedWindow,
) -> Vec<&'a crate::engine::PxText> {
    let story_bottom = story.y_px as i32 + story.h_px as i32;
    runs.iter()
        .copied()
        .filter(|t| {
            let py = t.y.max(1) as i32 - 1;
            py >= story_bottom
        })
        .collect()
}

/// How many terminal rows that strip needs: the span of its own GAME text rows.
///
/// This is the whole of SQ-0765's principle. Hybrid draws chrome text as text, one
/// game row per terminal row ([`draw_chrome_text_strip`] packs them that way,
/// SQ-0543), so the menu's height in rows is a property of the MENU — fixed, and
/// derived from its native pixel height — not of whatever the letterbox happened to
/// leave under the story.
fn menu_band_rows(menu: &[&crate::engine::PxText], cell: zvm::screen::V6Cell) -> u16 {
    // The v6 text cell is 8x16 (SQ-0479).
    let font_h = i32::from(cell.h());
    let rows: Vec<i32> = menu.iter().map(|t| (t.y.max(1) as i32 - 1) / font_h).collect();
    match (rows.iter().min(), rows.iter().max()) {
        (Some(&a), Some(&b)) => (b - a + 1).clamp(0, u16::MAX as i32) as u16,
        _ => 0,
    }
}

/// The bottom-anchored command strip's own strips (SQ-0765). It is a TEXT band by
/// construction — [`hybrid_bottom_plan`] picks `Menu` precisely when what lies below
/// the story window is runs and no art band — so it is one `Text` strip carrying
/// every one of those runs, and the cell path lays them out from the band's top.
///
/// Deliberately NOT [`decompose_chrome_strips`]: that classifier places a run through
/// the letterbox scale, which spreads N game rows over more than N terminal rows and
/// so cannot agree with a band sized by the rows themselves. Asking it here would
/// leave run-less gaps inside the band (Art strips redrawing squashed slices of the
/// frame's edge, SQ-0548) and, where the spread runs past the band's last row, drop
/// the runs that landed outside it — the menu's own last line.
fn menu_band_strips(
    bands: &[Rect],
    story: &crate::engine::PositionedWindow,
    runs: &[&crate::engine::PxText],
) -> Vec<ChromeStrip> {
    let menu = menu_band_runs(runs, story);
    bands.iter().map(|b| ChromeStrip::Text(*b, menu.iter().map(|t| (*t).clone()).collect())).collect()
}

/// One terminal row's class within the chrome ring.
enum RowClass<'b> {
    Text(Vec<&'b crate::engine::PxText>),
    /// The row carries runs AND opaque frame art behind them — Zork Zero's banner
    /// labels. The runs ride along (SQ-0944) so a backend that can put a glyph in a
    /// cell its artwork covers can draw them as characters instead of pixels; the
    /// strip is still an Art strip either way, because the art still has to reach
    /// the screen as an image.
    Art(Vec<&'b crate::engine::PxText>),
    /// No runs and no opaque frame art behind — bare background.
    Empty,
    /// A secondary prose window owns this row; the ring must not draw here.
    Panel,
}

/// What the chrome contains on a given terminal row — the one rule, asked in the
/// two places that need it (SQ-0894).
///
/// [`decompose_chrome_strips`] has always classified the rows of a full-width band
/// this way, to decide Text-as-glyphs vs. Art-as-pixels. The content-built ring
/// needs the SAME answer earlier and over the whole pane, to decide how far a flank
/// may extend: a flank may own a row of art, and must never own a row carrying
/// chrome text (its columns would swallow the ends of a full-width run) or a row a
/// secondary prose window draws itself.
///
/// It is one struct rather than two copies of three predicates because §6 of the
/// pipeline document lists "two places deciding the same thing by different rules"
/// as a defect class this file already suffers from; adding a fourth instance to
/// fix the ring would be a poor trade.
struct ChromeRowOracle<'a, 'b> {
    pane: Rect,
    scale: &'a crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    /// The GAME's Version 6 character cell (SQ-0917) — not [`Self::cell_px`], which
    /// is the terminal's. Every native-pixel-to-row step below divides by this one.
    v6_cell: zvm::screen::V6Cell,
    /// The native rect the terminal story viewport was cut from — the story window
    /// reduced to what the art leaves it (`story_text_native`, SQ-0896), not the
    /// declared window box. A run between the two is on the ring's side of the
    /// boundary now, so this is the rect "above or below the story" has to mean.
    story_native: (u32, u32, u32, u32),
    overlay_bottom: i32,
    panels: &'a [Rect],
    gfx: &'a image::RgbaImage,
    runs: &'a [&'b crate::engine::PxText],
    /// The NATIVE rows [`full_width_flood_rows`] calls bars — a run's row is in here
    /// when every run on it belongs to a grid window spanning ≥90% of the game
    /// screen and the game either asked for reverse video or the window is a status
    /// STRIP. SQ-0515's own rule, reused rather than restated (SQ-0894).
    bar_rows: &'a std::collections::HashSet<u16>,
    /// The native `[x0, x1)` columns the chrome WINDOWS contributing runs to each
    /// native text row span between them — a ribbon's own reach (SQ-0949).
    row_spans: &'a std::collections::HashMap<u16, (u16, u16)>,
}

impl<'b> ChromeRowOracle<'_, 'b> {
    /// A run sits over opaque frame art when its glyph span overlaps graphics.
    fn over_art(&self, t: &crate::engine::PxText) -> bool {
        let px0 = t.x.max(1) as u32 - 1;
        let py = t.y.max(1) as u32 - 1;
        let w = self.v6_cell.run_px(&t.text).max(u32::from(self.v6_cell.w()));
        crate::render::v6_layout::region_has_opaque(self.gfx, px0, py, w, u32::from(self.v6_cell.h()))
    }

    /// A run is a text-band candidate when it lies fully above or below the story
    /// box (never beside it — those stay in the side bands' ring).
    /// …plus a status strip the game OVERLAYS on the story box (SQ-0582): its runs
    /// are inside the box by native coordinates, but the caller has reserved their
    /// rows out of the story viewport, so they belong to the band like any other bar.
    fn below_or_above(&self, t: &crate::engine::PxText) -> bool {
        let story_top = self.story_native.1 as i32;
        let story_bottom = (self.story_native.1 + self.story_native.3) as i32;
        let py = t.y.max(1) as i32 - 1;
        // SQ-1020: the run's BOTTOM, from the game's own cell. Written `py + 16`
        // this asked whether a bar clears the story on a machine whose cell is
        // 16 — and a Macintosh bar sitting directly above the story misses by one
        // pixel at 15, drops out of the text band, and rasterises.
        let bottom = self.v6_cell.bottom_px(t.y) as i32;
        py >= story_bottom
            || bottom <= story_top
            || (self.overlay_bottom > 0 && bottom <= self.overlay_bottom)
    }

    fn runs_at(&self, row: u16) -> Vec<&'b crate::engine::PxText> {
        self.runs
            .iter()
            .copied()
            .filter(|t| self.below_or_above(t) && run_cell(t, self.scale, self.cell_px, self.pane, self.v6_cell).1 == row as i32)
            .collect()
    }

    fn in_panel(&self, row: u16) -> bool {
        self.panels.iter().any(|p| row >= p.y && row < p.bottom())
    }

    fn class(&self, row: u16) -> RowClass<'b> {
        if self.in_panel(row) {
            return RowClass::Panel;
        }
        let rr = self.runs_at(row);
        if rr.is_empty() {
            RowClass::Empty
        } else if rr.iter().any(|t| self.over_art(t)) {
            RowClass::Art(rr)
        } else {
            RowClass::Text(rr)
        }
    }

    /// Does the native region behind this CELL rect carry opaque art?
    ///
    /// The device→native inverse of the ring's letterbox scale, asked of a rect
    /// rather than of a row, because SQ-0750's rule is to classify a strip by what
    /// is IN it and not by where it sits.
    ///
    /// **Shared with the caller's art test, and now actually shared** (SQ-1059).
    /// This sentence used to promise it while `strip_has_art` carried a
    /// byte-identical copy of the body — SQ-0894 pasted it in rather than calling
    /// it, in a commit whose own message said "the fix should not add a fourth
    /// instance of it". The two never diverged, which is the only reason nothing
    /// was mis-rendered; SQ-1020 is what happens in this same file when a pair like
    /// that finally does. `strip_has_art` is now a one-line call to this.
    ///
    /// One other implementation of the question remains, deliberately:
    /// [`Self::over_art`] asks it in EXACT native coordinates rather than through
    /// this truncating inverse. §6 finding 4 of the pipeline document is the open
    /// question of whether those two can disagree at a boundary; settling it needs
    /// a boundary-crafted fixture and is not this quest.
    fn region_has_art(&self, r: Rect) -> bool {
        let cw = self.cell_px.0.max(1) as f32;
        let ch = self.cell_px.1.max(1) as f32;
        let s = self.scale.s.max(0.001);
        let inv_x = |c: u16| {
            (((c.saturating_sub(self.pane.x)) as f32 * cw - self.scale.off_x as f32) / s).max(0.0) as u32
        };
        let top = ((r.y.saturating_sub(self.pane.y)) as f32 * ch - self.scale.off_y as f32) / s;
        let bot = ((r.bottom().saturating_sub(self.pane.y)) as f32 * ch - self.scale.off_y as f32) / s;
        let y0 = top.max(0.0) as u32;
        let h = (bot.max(0.0) as u32).saturating_sub(y0).max(1);
        let x0 = inv_x(r.x).min(self.gfx.width());
        let x1 = inv_x(r.right()).min(self.gfx.width()).max(x0);
        crate::render::v6_layout::region_has_opaque(self.gfx, x0, y0, (x1 - x0).max(1), h)
    }

    /// The pane columns a run's glyphs occupy — `[first, last_exclusive)`.
    ///
    /// One terminal column per character, from the run's scale-mapped origin: the
    /// same two rates [`draw_chrome_text_strip`] stamps at, so "does this run stand
    /// in those columns" is asked here exactly as the draw answers it.
    fn run_cols(&self, t: &crate::engine::PxText) -> (i32, i32) {
        let (c, _) = run_cell(t, self.scale, self.cell_px, self.pane, self.v6_cell);
        (c, c + t.text.chars().count().max(1) as i32)
    }

    /// Does this run stand in `cols`?
    fn reaches(&self, t: &crate::engine::PxText, cols: (u16, u16)) -> bool {
        let (c0, c1) = self.run_cols(t);
        c1 > cols.0 as i32 && c0 < cols.1 as i32
    }

    /// Chrome TEXT on `row` that stands in `cols` — the only text that can stop a
    /// flank from owning the row (SQ-0894).
    ///
    /// The rule used to be "any chrome text anywhere on this row", which is a
    /// statement about the row and not about the flank. MEASURED on Shogun's
    /// credits screen (`shogun-r322-s890706.z6`, release 322): the nine credit runs
    /// are CENTRED at native x 105..537 and the frame's ornament columns are native
    /// 0..46 and 594..640, so the two never meet — yet the row veto cut the flank in
    /// two and left the ornaments as `8x3` and `8x24` with the credits' ten rows
    /// between them.
    ///
    /// A row whose runs sit OVER art is not text at all (`class` says `Art`) and has
    /// never blocked anything; that stays true.
    ///
    /// A BAR blocks whatever its glyphs do, **as far as its own WINDOW reaches and no
    /// further** (SQ-0515, bounded by SQ-0949). `full_width_flood_rows` is the
    /// existing, derived answer to "is this row a ribbon the game draws edge to edge
    /// or a block of text standing in the middle of the screen", and the distinction
    /// it already makes is exactly the one a flank needs: Arthur's status window is
    /// 584 of 640 native columns and reads as one solid ribbon whose glyphs stop well
    /// short of both ends, so it reaches past them even though its text does not;
    /// Shogun's credits are a 432-column block and reach nothing.
    ///
    /// SQ-0949 is the other end of that sentence. Read as "a bar owns the whole row",
    /// the clause gave Arthur's ribbon the pane's full width — flooding the strip's
    /// ground straight across both poles and cutting each flank into a piece above the
    /// bar and a piece below it, which is the step the report describes as the side
    /// strip not lining up with the panel above it. The bar's window is native
    /// `28..612` of 640 and stops there on the machine: measured on
    /// `machine-screenshots/dos-arthur.png` (the EGA press at the Churchyard, "Merlin
    /// disappears"), the white ribbon spans native **28..610** and the grey pole rule
    /// stands beside it at native **6.5..8.7**, unbroken from the panel's foot to the
    /// screen's; `machine-screenshots/mac-arthur.png` shows the same frame with the
    /// black ribbon inset and the green poles running past both of its ends.
    ///
    /// So the reach is the window's span quantized INWARD — the cells a strip's ground
    /// can honestly claim, `ceil` on the left and `floor` on the right, the same
    /// rounding the viewport takes. That keeps the ribbon solid wherever its window
    /// really is edge to edge and hands the sub-cell remainder back to the flank,
    /// which is where the artwork is.
    fn blocked(&self, row: u16, cols: (u16, u16)) -> bool {
        match self.class(row) {
            RowClass::Text(rr) => rr.iter().any(|t| {
                let native_row = self.v6_cell.row_of(t.y);
                // A BAR reaches as far as its own WINDOW and no further (SQ-0949) —
                // and only a bar takes this branch. A row of ordinary chrome text is
                // still judged by where its GLYPHS stand: the window a run sits in
                // says nothing about a block of credits centred inside it, and
                // Shogun's own status band is 548 of 640 native columns, under the
                // bar threshold, with its first glyph straddling the flank's last
                // cell — measured on `James Clavell's Shogun.adf` (release 295) at a
                // 70-column pane, judging that row by its window instead lost the
                // `E` of `Erasmus:` to the flank.
                if self.bar_rows.contains(&native_row) {
                    return self.row_spans.get(&native_row).is_none_or(|&s| self.span_reaches(s, cols));
                }
                self.reaches(t, cols)
            }),
            _ => false,
        }
    }

    /// Does a native `[x0, x1)` span, quantized inward to whole pane cells, stand in
    /// `cols`? See [`blocked`](Self::blocked) — this is a bar's reach, as against
    /// [`reaches`](Self::reaches), which is one run's glyphs.
    fn span_reaches(&self, native: (u16, u16), cols: (u16, u16)) -> bool {
        let cw = self.cell_px.0.max(1) as f32;
        let dev = |n: u16| (self.pane.x as f32 * cw + self.scale.off_x as f32 + n as f32 * self.scale.s) / cw;
        let c0 = dev(native.0).ceil() as i32;
        let c1 = dev(native.1).floor() as i32;
        c1 > cols.0 as i32 && c0 < cols.1 as i32
    }

    /// SQ-0508's bridge, asked of the whole pane rather than of one band: an EMPTY
    /// row whose nearest non-empty neighbours above AND below are both chrome TEXT
    /// belongs to that text panel. It is a blank row the letterbox scale opened
    /// between two printed rows, not a row of frame.
    ///
    /// MEASURED on Shogun's gameplay band: its two status lines land on terminal
    /// rows 0 and 2 at a large pane, and row 1 carries no runs. Letting a flank own
    /// row 1 split the band in two, and the second line moved from row 1 to row 2 —
    /// "Erasmus/SHOGUN/Score" on one row, halfblock frame art across the next, and
    /// "Bridge/Moves" below that.
    ///
    /// SQ-0894: the bridge follows the same relaxation as the veto it serves — a
    /// gap row is only bridged away from a flank when the text rows it sits between
    /// are text this flank must yield to. A blank row between two rows of CENTRED
    /// credits is, in the ornament's own columns, just more ornament.
    fn bridged(&self, row: u16, cols: (u16, u16)) -> bool {
        if !matches!(self.class(row), RowClass::Empty) {
            return false;
        }
        let neighbour = |mut r: u16, up: bool| -> bool {
            loop {
                if up {
                    if r <= self.pane.y {
                        return false;
                    }
                    r -= 1;
                } else {
                    r += 1;
                    if r >= self.pane.bottom() {
                        return false;
                    }
                }
                if !matches!(self.class(r), RowClass::Empty) {
                    return self.blocked(r, cols);
                }
            }
        };
        neighbour(row, true) && neighbour(row, false)
    }

    /// Whether a FLANK spanning `cols` may own `row`: the row must belong to no
    /// prose panel, carry no chrome TEXT IN THOSE COLUMNS (nor be a blank row
    /// between two such text rows), AND this flank's own columns must actually have
    /// art on it.
    ///
    /// The art half is not redundant. Without it a flank extends over bare ground,
    /// which changes the rect `flank_borders` scans for the frame's border columns
    /// and so changes what it finds. MEASURED on Journey's Amiga press at a 163x61
    /// pane: letting the flank take one run-less row above the viewport made the
    /// border pass report THREE dividers instead of two — a band-ink rule at col
    /// 162 drawn beside the real `│` glyph at col 163, which is the doubled rule
    /// `journey_flank_border_is_drawn_at_the_letterbox_scale` exists to forbid.
    fn flankable(&self, row: u16, cols: (u16, u16)) -> bool {
        if matches!(self.class(row), RowClass::Panel) {
            return false;
        }
        if self.blocked(row, cols) || self.bridged(row, cols) {
            return false;
        }
        cols.1 > cols.0 && self.region_has_art(Rect::new(cols.0, row, cols.1 - cols.0, 1))
    }
}

/// The pane columns the frame's own SIDE ARTWORK occupies at each edge, as
/// `(left_end, right_start)` in pane-absolute cells (SQ-0894).
///
/// Step (a) of this quest made a flank's ROWS content-derived and left its COLUMNS
/// as `pane.x..viewport.x` and `viewport.right()..pane.right()` — still "whatever
/// the story box leaves". That is why the same title, the same medium and the same
/// artwork can have flanks or not depending only on where the game put window 0:
/// MEASURED on `James Clavell's Shogun.adf` (release 295, serial 890321), window 0
/// is `548x368 at (47,33)` in play and `640x64 at (0,336)` at the credits screen, so
/// `pane − viewport` has side rects in one and none in the other. The ornaments are
/// the same ornaments either way.
///
/// So ask the art. For each native row, take the first CONTIGUOUS opaque run in from
/// each edge, and keep the NARROWEST end over all such rows. Narrowest, because a
/// side column is as wide as the art is where it is narrowest — a banner or a
/// capital above it is wider (Zork Zero's banner spans all 640 columns; Arthur's
/// header spans 28..612 with the poles beside it) and must not be mistaken for the
/// column hanging below. It is the same reduction `v6_border::painted_widths` makes
/// to tell a pillar from a slab, read from the pane edge instead of as a span.
///
/// MEASURED across the corpus at 98x37 / 8x18 (native px, then cells):
///
/// | frame | left run-end | right run-start | cells | today's flank |
/// |---|---|---|---|---|
/// | Zork Zero | 62 | 580 | 10 / 88 | 14 / 85 |
/// | Arthur, in play | 10 | 630 | 2 / 96 | 5 / 94 |
/// | Shogun, in play | 46 | 594 | 8 / 90 | 8 / 90 |
/// | Shogun, credits (Blorb r322) | 46 | 594 | 8 / 90 | 8 / 90 |
/// | Shogun, credits (Amiga r295) | 46 | 594 | 8 / 90 | **none** |
/// | Journey | 226 | — | 35 / — | 37 / 97 |
///
/// The caller takes the WIDER of this and the story box's leftover, which is why
/// every row of that table except the Amiga credits screen leaves the corpus exactly
/// where it stands: art that stops short of the story box does not shrink a flank
/// (the cells between the two are chrome and the flank is still their owner), and art
/// that runs past it grows one. Under-claiming is therefore safe and over-claiming is
/// not, which is the whole reason the statistic is a minimum.
///
/// SQ-0899: and the run must REACH the edge it is claimed for. "The first opaque run
/// in from the edge" is not the same sentence as "a column of side artwork **at the
/// pane's edge**", which is what the caller asks for and what a flank is; a picture
/// standing alone in the middle of the screen answers the first and is not the
/// second. MEASURED as the closest ink to either edge, native px, on the frame each
/// title shows after its intro:
///
/// | frame | nearest the left edge | nearest the right |
/// |---|---|---|
/// | Zork Zero (r393) | **0** | **639** |
/// | Shogun (r322 Blorb, r295 Amiga) | **0** | **639** |
/// | Arthur (r74 IBM, r54 Amiga) | 6 | 633 |
/// | Journey (r83 IBM / r77 ProDOS / r30 Amiga) | 6 / 7 / 22 | — |
/// | **Arthur (r63 ProDOS, `Arthur.po` and the 2mg)** | **250** | **389** |
///
/// "Reaches" is within one native text CELL, which is the allowance `seek_ink` makes
/// for the same reason a column over: a frame is authored on the game's text grid and
/// its ink may sit anywhere inside its own eight pixels, so Arthur's four-pixel gutter
/// is a pole at the edge and not a picture away from it. Nothing in the corpus needs a
/// finer judgement than that. The ProDOS Arthur is not close to the line: its frame is
/// a centred illustration painting native columns 250..389 and NOTHING else, so the
/// run in from each edge was the same picture, read from both sides. That made each
/// flank 253 native px — 39 cells at a 98x37 pane, 67 at 169x62 — and `flank_source`
/// then TILED the sliver of picture inside it down the whole column, which is the
/// banner repeating down the flank that quest reports. Requiring contact costs the
/// corpus nothing: the two frames whose art runs past the story box (Zork Zero's and
/// Shogun's) touch their edges exactly, and every other title's claim was already the
/// narrower of the two the caller maxes, so it never decided a flank.
fn flank_art_columns(
    gfx: &image::RgbaImage,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    cell: zvm::screen::V6Cell,
) -> (u16, u16) {
    let (w, h) = (gfx.width(), gfx.height());
    let mid = w / 2;
    if mid == 0 || h == 0 {
        return (pane.x, pane.right());
    }
    let opaque = |x: u32, y: u32| gfx.get_pixel(x, y)[3] >= 128;
    let (mut left, mut right): (Option<u32>, Option<u32>) = (None, None);
    for y in 0..h {
        if let Some(first) = (0..mid).find(|&x| opaque(x, y)) {
            let mut end = first;
            while end < mid && opaque(end, y) {
                end += 1;
            }
            left = Some(left.map_or(end, |m: u32| m.min(end)));
        }
        if let Some(last) = (mid..w).rev().find(|&x| opaque(x, y)) {
            let mut start = last + 1;
            while start > mid && opaque(start - 1, y) {
                start -= 1;
            }
            right = Some(right.map_or(start, |m: u32| m.max(start)));
        }
    }
    // SQ-0899, the two ways a run is not a side column. The reduction above stays over
    // EVERY row in both cases — narrowing it to the rows that qualify would re-admit
    // the banner it exists to exclude, and takes Zork Zero's left column from 62
    // native px to 72 — so each is a separate question asked of the edge afterwards.
    //
    // It REACHED THE MIDDLE, where the scan is bounded because a flank may not pass
    // it. Then the bound is what stopped the run, not the art, and the measurement
    // says nothing about a column's width. mysterious01's frame is one picture over
    // the whole screen: both runs come back at the middle, which is not two flanks
    // meeting but one image, and it was only ever harmless because the caller's own
    // `r > x` test threw the pair away — a symmetry the contact test below breaks the
    // moment it answers for one edge and not the other.
    //
    // It NEVER TOUCHED THE EDGE it is claimed for (see the table above).
    let font_w = u32::from(cell.w());
    let reaches = |from: u32, inward: i32| {
        (0..h).any(|y| (0..font_w as i32).any(|d| opaque((from as i32 + d * inward) as u32, y)))
    };
    let (left, right) = (
        left.filter(|&e| e < mid).filter(|_| reaches(0, 1)),
        right.filter(|&s| s > mid).filter(|_| reaches(w - 1, -1)),
    );
    // Native → pane cells through the ring's own letterbox scale, rounded OUTWARD
    // so a flank covers the art's last pixel rather than clipping it, and never
    // past the pane's middle: two flanks meeting would leave no screen at all.
    let cw = cell_px.0.max(1) as f32;
    let dev = |n: u32| (scale.off_x as f32 + n as f32 * scale.s) / cw;
    let half = pane.x + pane.width / 2;
    let to_col = |v: f32| pane.x.saturating_add(v.clamp(0.0, pane.width as f32) as u16);
    (
        left.map_or(pane.x, |n| to_col(dev(n).ceil()).clamp(pane.x, half)),
        right.map_or(pane.right(), |n| to_col(dev(n).floor()).clamp(half, pane.right())),
    )
}

/// The chrome ring carved by CONTENT rather than as `pane − viewport` (SQ-0894).
///
/// [`v6_layout::chrome_bands`] gives each flank only the story viewport's vertical
/// extent, because the ring is *defined* as the viewport's complement. A flank
/// column is therefore composed of up to three pieces drawn by two different
/// routines off two different canvases. MEASURED on Zork Zero at a 98x37 pane,
/// 8x18 cell, kitty: rows 1..6 come from the full-width top band's tiles, cropping
/// the frame-shared scaled canvas at magnification 1.2250/1.2250; rows 7..37 come
/// from the side band, resampling the EXTENDED flank source 91x456 into 112x558 —
/// 1.2308 horizontally, 1.2237 vertically. Half a pixel of shear at the join,
/// invisible today only because both pieces happen to read the same continuous
/// artwork across it.
///
/// Here a flank owns every contiguous row it MAY own — art, or bare ground —
/// stopping at any row carrying chrome TEXT or belonging to a secondary prose
/// panel. Those two exclusions are the whole of the rule, and both were measured
/// rather than guessed:
///
/// - Arthur's status bar is a full-width `strip:text(72 runs)` between his header
///   art and his story window. A flank spanning it would take the outer five
///   columns of a single text row into flank ART, cutting the run three ways.
/// - Journey's command menu is the bottom band under the `Menu` plan. A flank
///   spanning it would swallow the left 37 columns of the verb menu.
///
/// So the corpus lands: Zork Zero's flank becomes rows 1..37 CONTINUOUS and its
/// seam ceases to exist; Arthur's becomes 1..13 and 15..37, split at his status
/// bar rather than at the arbitrary edge of the story window; Shogun (3..37) and
/// Journey (1..31) are already optimal and must not move.
///
/// The top/bottom bands keep the full pane width on rows the flanks did not take
/// and narrow to the viewport's columns on rows they did, so no cell is drawn
/// twice and the tiling of `pane − viewport` stays exact.
fn content_ring_bands(
    pane: Rect,
    viewport: Rect,
    menu_plan: bool,
    oracle: &ChromeRowOracle,
) -> Vec<(crate::render::v6_layout::BandRole, Rect)> {
    use crate::render::v6_layout::BandRole;
    let vx = viewport.x.clamp(pane.x, pane.right());
    let vy = viewport.y.clamp(pane.y, pane.bottom());
    let vr = viewport.right().clamp(vx, pane.right());
    let vb = viewport.bottom().clamp(vy, pane.bottom());

    // No flank to build: the viewport spans the pane's width, so there is nothing
    // for a side column to own and the ring is exactly the old top/bottom pair.
    // mysterious01, fmvpoker and advent are all this shape at a 98-column pane.
    if vx <= pane.x && vr >= pane.right() {
        return crate::render::v6_layout::chrome_bands(pane, viewport);
    }

    // Which rows a flank may own. Rows level with the viewport always qualify —
    // that is the band the old ring already drew — and the rows above and below it
    // qualify by content.
    // …and under the `Menu` plan nothing below the viewport qualifies at all: that
    // band IS the game's own command strip, spanning the pane by construction, and
    // it is split off whole a few lines below. MEASURED on Journey — letting the
    // rule decide row by row handed the flank rows 32..37, because its menu text is
    // painted over artwork and so reads as Art; the strip came back
    // `menu:text(22 runs) 59x6 at (38,32)` instead of 98x6 and the verb menu lost
    // its left 37 columns to black.
    let own = |cols: (u16, u16), row: u16| -> bool {
        if menu_plan && row >= vb {
            return false;
        }
        if (vy..vb).contains(&row) {
            return true;
        }
        oracle.flankable(row, cols)
    };
    let left_cols = (pane.x, vx);
    let right_cols = (vr, pane.right());
    let mut owned: Vec<(bool, bool)> = (pane.y..pane.bottom())
        .map(|row| (own(left_cols, row), own(right_cols, row)))
        .collect();
    // A FRAME's two flanks own the same rows. Where the two scans disagree, neither
    // takes the row.
    //
    // The scans can disagree for a reason that has nothing to do with the artwork,
    // and on the corpus that is the only reason they ever do. Shogun's status band
    // sits at native x 46..594 — exactly between his two ornaments — and its first
    // glyph is at native 49. At 98x37 the left ornament ends at 7.04 cells and that
    // glyph lands at 7.35: the SAME terminal column, so the text wins it and the left
    // flank yields the row, while the right flank (whose last run ends clear of it)
    // takes it. MEASURED: the right ornament ran to the pane's first row level with
    // Score/Moves and the left started three rows down, so one top corner carried
    // ornament and the other bare ground under the band's flood. The frame read as
    // lopsided on a screen that was already right.
    //
    // Intersecting is deliberately the conservative repair, and it cannot cost
    // anything that ever shipped: before this quest NO row outside the story
    // viewport's own span belonged to a flank at all, and rows inside that span are
    // unconditionally owned by both sides above. So the intersection can only give
    // back rows step (a) added, never take away rows the old ring drew. The corpus
    // agrees on every frame either way — Zork Zero, Arthur, Journey and both Shogun
    // credits presses all have their two flanks owning identical row sets — so this
    // moves exactly the one frame it is for.
    //
    // Only when BOTH sides have columns: a frame with one flank (or none) must not
    // have its single ornament intersected against an empty side.
    if left_cols.1 > left_cols.0 && right_cols.1 > right_cols.0 {
        for o in &mut owned {
            let both = o.0 && o.1;
            *o = (both, both);
        }
    }
    let at = |row: u16| owned.get((row - pane.y) as usize).copied().unwrap_or((false, false));

    let mut out: Vec<(BandRole, Rect)> = Vec::new();

    // One flank rect per maximal run of rows that side owns — the flank is ONE
    // object over each run, not one piece per band edge. Each side still runs its own
    // scan: the two flanks differ in WIDTH even where they agree on rows, and on
    // Journey they differ by a lot (its left flank is a half-screen picture column,
    // its right eight native pixels of border).
    for (role, cols, pick) in [
        (BandRole::LeftFlank, left_cols, 0usize),
        (BandRole::RightFlank, right_cols, 1usize),
    ] {
        if cols.1 <= cols.0 {
            continue;
        }
        let mine = |row: u16| { let o = at(row); if pick == 0 { o.0 } else { o.1 } };
        let mut r = pane.y;
        while r < pane.bottom() {
            if !mine(r) {
                r += 1;
                continue;
            }
            let start = r;
            while r < pane.bottom() && mine(r) {
                r += 1;
            }
            out.push((role, Rect::new(cols.0, start, cols.1 - cols.0, r - start)));
        }
    }

    // Top and bottom, split by whether the flanks took the row: each SIDE gives up
    // its own columns where its own flank took the row, and keeps them where it did
    // not. Asked per side rather than "either side took it", because the two scans
    // need not agree — Journey's left flank is a half-screen picture column and its
    // right is eight native pixels of border, and a row one owns and the other does
    // not would otherwise leave the other side's columns with no owner at all.
    let band_runs = |role: BandRole, y0: u16, y1: u16, out: &mut Vec<(BandRole, Rect)>| {
        let mut r = y0;
        while r < y1 {
            let taken = at(r);
            let start = r;
            while r < y1 && at(r) == taken {
                r += 1;
            }
            let x0 = if taken.0 { vx } else { pane.x };
            let x1 = if taken.1 { vr } else { pane.right() };
            if x1 > x0 {
                out.push((role, Rect::new(x0, start, x1 - x0, r - start)));
            }
        }
    };
    band_runs(BandRole::Top, pane.y, vy, &mut out);
    band_runs(BandRole::Bottom, vb, pane.bottom(), &mut out);
    out
}

/// Carve the hybrid chrome `bands` into drawable strips (SQ-0500). Narrow side
/// bands (beside the story viewport) stay one `Art` strip — picture columns and
/// borders. Each FULL-WIDTH band (top/bottom of the ring) is split row-by-row:
/// a terminal row is a TEXT row when it carries chrome runs that lie OUTSIDE the
/// story box, above or below it, with NO opaque frame graphics behind them; every
/// other row is ART. Consecutive rows of one class merge into a strip. `story` is
/// the story window (its native pixel box splits above/below); `gfx` is the
/// graphics-only chrome canvas (the art test, via [`region_has_opaque`]).
///
/// `panels` are the cell rects of SECONDARY PROSE windows (SQ-0585) — a v6 game's
/// second scrolling text window, which the renderer draws as terminal text of its
/// own. Those rows belong to that window, so no strip is emitted for them at all:
/// classing them ART made the ring rasterize a slice of the chrome canvas straight
/// over the panel, and under a graphics protocol like kitty the image composites
/// ABOVE the cells, so the panel's text vanished behind stray rasterized banner.
///
/// Returns the strips, and beside them every run that landed on an ART row —
/// text the game printed ON its artwork (SQ-0944). Those runs are drawn as pixels
/// in the chrome canvas as they always were; a backend that can put a glyph in a
/// cell an image covers uses this list to draw them as characters instead, which
/// is the difference between legible and unreadable on half-blocks. The strips
/// themselves are unchanged: the art still ships as an image either way.
fn decompose_chrome_strips<'a>(
    bands: &[(crate::render::v6_layout::BandRole, Rect)],
    oracle: &ChromeRowOracle<'_, 'a>,
) -> (Vec<ChromeStrip>, Vec<&'a crate::engine::PxText>) {
    let mut out = Vec::new();
    let mut over_art: Vec<&'a crate::engine::PxText> = Vec::new();
    for (role, band) in bands {
        // A FLANK is never text — one Art strip. Asked by role since SQ-0894: the
        // test used to be `band.width < pane.width`, which answers correctly only
        // while the top and bottom bands span the whole pane. Measured on the two
        // frames that would have broken first: Shogun's header is a full-width
        // `strip:text(8 runs)` and Arthur's status bar a full-width
        // `strip:text(72 runs)`, both on plain ground and both rendered as glyphs
        // today — a width test would have classed either as Art the moment a
        // top band stopped spanning the pane, rasterising what the game printed as
        // characters (SQ-0750). Zork Zero's banner is unaffected either way: it is
        // text OVER art, the legitimate raster case, and `over_art` decides it.
        if role.is_flank() {
            out.push(ChromeStrip::Art(*role, *band));
            continue;
        }
        // Classify each terminal row of this full-width band.
        let classes: Vec<RowClass> = (band.y..band.bottom()).map(|row| oracle.class(row)).collect();
        // SQ-0508: bridge a scale-introduced interior gap row into the menu panel.
        // When the letterbox scale spreads N native menu rows across N+ terminal
        // rows, a bare terminal row can fall BETWEEN two menu rows (Journey's
        // command menu: a blank row below the header, and one above "Tag"), breaking
        // the reversed vertical column dividers. An Empty row whose nearest non-Empty
        // neighbour above AND below are both Text is part of that panel → mark it Text
        // so the whole menu is one strip (continuous background + dividers). Empty
        // rows at an art boundary (Arthur's panel over the status) stay Art, so the
        // pixel ring keeps showing through there.
        let n = classes.len();
        let is_text = |c: &RowClass| matches!(c, RowClass::Text(_));
        let mut bridge = vec![false; n];
        for i in 0..n {
            if !matches!(classes[i], RowClass::Empty) {
                continue;
            }
            let above = (0..i).rev().find(|&j| !matches!(classes[j], RowClass::Empty));
            let below = (i + 1..n).find(|&j| !matches!(classes[j], RowClass::Empty));
            if above.is_some_and(|j| is_text(&classes[j])) && below.is_some_and(|j| is_text(&classes[j])) {
                bridge[i] = true;
            }
        }
        // Coalesce consecutive same-class (Text|bridged vs. not) rows into strips.
        let mut i = 0usize;
        while i < n {
            // A panel's rows produce no strip: that window draws itself.
            if matches!(classes[i], RowClass::Panel) {
                i += 1;
                continue;
            }
            let text = matches!(classes[i], RowClass::Text(_)) || bridge[i];
            let mut j = i;
            let mut text_runs: Vec<&crate::engine::PxText> = Vec::new();
            while j < n
                && !matches!(classes[j], RowClass::Panel)
                && (matches!(classes[j], RowClass::Text(_)) || bridge[j]) == text
            {
                match &classes[j] {
                    RowClass::Text(rr) => text_runs.extend(rr.iter().copied()),
                    // A bridged Art row is folded into a TEXT strip, and a text strip
                    // draws its own runs — so only the rows that really stay Art
                    // contribute here, or the same run would be stamped twice.
                    RowClass::Art(rr) if !text => over_art.extend(rr.iter().copied()),
                    _ => {}
                }
                j += 1;
            }
            let rect = Rect::new(band.x, band.y + i as u16, band.width, (j - i) as u16);
            out.push(if text {
                ChromeStrip::Text(rect, text_runs.iter().map(|t| (*t).clone()).collect())
            } else {
                ChromeStrip::Art(*role, rect)
            });
            i = j;
        }
    }
    (out, over_art)
}

/// How many terminal columns one uploaded chrome-band image covers (SQ-0818).
///
/// A full-width ring band used to be ONE image, so any change inside it re-encoded and
/// re-transmitted the whole thing: Zork Zero's banner is 920x126 device pixels — 618 KB
/// of base64 in 151 kitty chunks — and it went down the wire in full eight times
/// during boot, ~4.9 MB, because one 45x40 compass tile changed each time. Neither
/// kitty nor iterm2 has an op to patch pixels into a resident image, so the portable
/// way to send only the dirty region is to make the images smaller.
///
/// Eight columns, from the arithmetic rather than from taste. At the captured 8x18
/// cell that banner is 115 cells wide; a tile is `T*8*126*4` bytes of RGBA, i.e.
/// `T*5376` base64 characters, and kitty takes 4096 of those per chunk — so each tile
/// wastes half a chunk (~2 KB) on its rounded-up last one, and each costs one extra
/// control block. Against that, the compass spans about 8 cells, so it dirties
/// `ceil(8/T)+1` tiles at worst:
///
/// | T  | tiles | first frame | compass re-send |
/// |----|-------|-------------|-----------------|
/// | 1  |  115  | +230 KB     |  ~48 KB         |
/// | 4  |   29  |  +58 KB     |  ~65 KB         |
/// | 8  |   15  |  +30 KB     |  ~86 KB         |
/// | 16 |    8  |  +16 KB     |  ~172 KB        |
///
/// Below 8 the fixed cost stops buying anything — T=4 saves 21 KB on a redraw and
/// pays 28 KB more on every first frame — and above it the re-send climbs fast. Eight
/// also keeps the resident image count per band in the teens rather than the hundreds,
/// which matters because a terminal evicts images by LRU (SQ-0753).
///
/// Columns only, not a grid: the ring's full-width bands are wide and shallow by
/// construction (115 x 7 cells here), so splitting the short axis at most doubles the
/// tile count for a change that, like the compass, usually spans the band's full
/// height anyway.
///
/// MEASURED, not modelled — `cargo run -p app --example pty_capture -- --story
/// stories/zork0-r393-s890714.z6 --size 117x64 --keys wait:400,wait:400,wait:400`, the
/// same three frames on either side of this constant:
///
/// |                   | one strip | 15 tiles | ratio |
/// |-------------------|-----------|----------|-------|
/// | first frame       | 2,089,630 | 2,093,195| +0.17% |
/// | compass, 1 tile   |   629,280 |   43,947 | 14.3x |
/// | compass, 1 tile   |   628,475 |   43,947 | 14.3x |
/// | compass, 2 tiles  |   628,566 |   88,349 | 7.1x  |
/// | compass, 2 tiles  |   628,473 |   88,252 | 7.1x  |
/// | whole capture     | 4,604,778 | 2,358,042| 1.95x |
///
/// The transmitted PAYLOAD on the first frame is byte for byte the same 618,240 —
/// 14 tiles of 43,008 plus one of 16,128 — because the tiles crop from the same scaled
/// canvas. The 3,565-byte difference is entirely per-image framing.
const BAND_TILE_COLS: u16 = 8;

/// Partition `band` into at most `tile_cols`-wide column tiles (SQ-0818), left to
/// right. `tile_cols == 0` disables tiling — see the caller for which backends ask
/// for that.
///
/// The partition is EXACT: the tiles' x-spans tile `[band.x, band.right())` with no
/// gap and no overlap, every tile is at least one cell wide, and every tile keeps the
/// band's own `y`/`height`. That is what makes the tiles pixel-identical to the strip
/// they replace: [`crate::render::graphics::GraphicsRender::draw_chrome_band`] crops
/// each band out of the ONE frame-shared scaled canvas (`chrome_scaled`, SQ-0514) at
/// whole device pixels — column `c` reads scaled pixels `[c*cw, (c+1)*cw)` however the
/// band around it is cut — so there is no resampling boundary at a tile seam and no
/// ceil-vs-round trap.
fn band_tiles(band: Rect, tile_cols: u16) -> Vec<Rect> {
    if tile_cols == 0 || band.width <= tile_cols {
        return vec![band];
    }
    let mut out = Vec::new();
    let mut x = band.x;
    while x < band.right() {
        let w = tile_cols.min(band.right() - x);
        out.push(Rect::new(x, band.y, w, band.height));
        x += w;
    }
    out
}

/// Paint a TEXT chrome strip (SQ-0500) as terminal cells: each run stamped at its
/// scale-mapped cell with [`v6_run_style`], clipped to `rect`. The strip and each
/// run row are flooded (colour-aware, SQ-0512) before the runs stamp, so the panel
/// reads as one solid block carrying the game's own background — not just cells
/// behind the glyphs. A PURE reverse-video row (a status/menu bar — every run
/// reversed) floods edge to edge reversed, so a bar the game painted as separate
/// runs with bare gaps reads as one solid block (SQ-0499 cell path): Arthur's
/// status row loses its lone unreversed cell; Journey's menu header bar closes the
/// gap between its two labels. Mixed rows (Journey's menu body — normal verb text
/// with reversed dividers) are not flood-reversed.
#[allow(clippy::too_many_arguments)]
/// Can this graphics backend show a terminal GLYPH in a cell its artwork covers?
///
/// A CAPABILITY, asked of the picker that actually negotiated (the same source
/// `examples/pty_capture`'s VERDICT line reads) rather than guessed from config —
/// SQ-0936's trap was an option that appeared to apply generally and quietly did
/// nothing on some paths, and the fix for that is to ask.
///
/// **Half-blocks: yes, and it is not a preference.** The backend draws `▀` with a
/// foreground and a background, so a cell is two vertical samples and a rasterised
/// 8x16 glyph arrives as 8x2 — not ugly, unreadable. Real glyphs are exact. That
/// makes it the difference between a usable and an unusable backend for terminals
/// with no graphics protocol at all, for tmux, and for asciinema casts, which
/// record exactly this backend because it is glyphs plus 24-bit SGR. There is no
/// reason a player would want the mush back, so there is no setting.
///
/// **Kitty: NO — and this is the part that was measured rather than assumed.**
/// SQ-0944 set out to stop rasterising here too, on the reading that every
/// placement sits at kitty's `z = -1` ("over the backgrounds but under the text")
/// and so a glyph would simply appear on top. The oracle says otherwise, and the
/// reason has nothing to do with z: lanthorn's placements are **virtual** (`U=1`),
/// positioned BY `U+10EEEE` placeholder characters, so the image IS the cell's
/// content and there is no glyph layer in that cell to be over anything. Printing
/// a character into a covered cell does not composite over the image, it DELETES
/// it — the cell loses the placement, and the rest of the row's run goes with it
/// (measured both ways: on the lead cell, which carries the diacritic triple, the
/// whole row dies; on a continuation cell, everything from there rightward does).
/// `pty_oracle::raster::a_glyph_printed_into_a_virtual_placement_erases_it` pins
/// it. Two further facts agree: `ratatui-image`'s `transmit_virtual` emits no `z`
/// parameter at all, and it marks every cell after a row's first `Skip`, so the
/// app could not address those cells even if the protocol allowed it. The z=-1 in
/// `pty_stream/raster.rs`'s note is what the RENDERER sorts virtual placements at
/// internally — that module says so — not what lanthorn asks for, and it governs
/// classic pin-anchored placements, which is what
/// `a_negative_z_placement_draws_under_the_text` covers.
///
/// **Sixel and iTerm2: no.** Neither has a Z index at all; their images become
/// cell content outright.
///
/// So the capability is absent on three of the four backends, for two different
/// reasons, and present on the one where it is not optional. That is why this is a
/// predicate and not a config key: there is nothing left for a player to choose.
fn backend_layers_glyphs_over_art(picker: &ratatui_image::picker::Picker) -> bool {
    matches!(picker.protocol_type(), ratatui_image::picker::ProtocolType::Halfblocks)
}

/// Whether this backend resolves an image's TRANSPARENCY to black (SQ-0944).
///
/// A different question from [`backend_layers_glyphs_over_art`], with the same
/// answer today and no reason to stay that way, so it is asked separately.
///
/// Half-blocks is the one that provably does: `ratatui-image`'s primitive
/// encoder calls `to_rgb8()`, which drops alpha and leaves a fully transparent
/// pixel at RGB 0,0,0 — and `pick_side` then collapses the two equal halves to a
/// SPACE, so the region reaches the screen as space cells on a black background.
/// That is the black gutter down both sides of Zork Zero's pillars, which under
/// kitty is the white page the story window declared.
///
/// Kitty and iTerm2 keep the alpha and composite it themselves (both ship the
/// pixels with an alpha channel — RGBA and PNG respectively), so their bands must
/// go on shipping it: flattening for them would paint the letterbox margin the
/// game's page instead of the terminal's own ground. Sixel hands `to_rgba8()` to
/// its encoder and has not been measured here; it keeps today's behaviour until
/// it is.
fn backend_flattens_alpha_to_black(picker: &ratatui_image::picker::Picker) -> bool {
    matches!(picker.protocol_type(), ratatui_image::picker::ProtocolType::Halfblocks)
}

/// The cells one run puts on screen, per glyph that landed inside an art strip:
/// its pane column and row, the character, and the NATIVE x of the declared cell
/// it was published at.
///
/// The native x rides along because the colour rule is asked per glyph (SQ-1060)
/// and the over-art probe needs game pixels, while the first two are pane cells.
/// Grouping by run remains, because a run's STYLE bits are the run's.
type StampedCells = Vec<(u16, u16, char, u32)>;

/// Stamp the runs a game printed ON its artwork as terminal glyphs, over the art
/// band that has already been drawn into these cells (SQ-0944).
///
/// Only reached on a backend [`backend_layers_glyphs_over_art`] approves, which
/// today means half-blocks. There is no z-order there — text and art share one
/// cell — so a glyph must carry a background, and the right one is the art behind
/// it. It is sampled from the BUFFER rather than re-derived from the chrome canvas
/// on purpose: the band has already resolved the whole device→native mapping and
/// written the answer into `cell.fg` (the upper half) and `cell.bg` (the lower),
/// so reading it back is exact by construction and adds no fourth implementation
/// of an inverse whose rounding is where v6 geometry bugs live. The two halves are
/// each a mean of their half, so their mean is the mean of the cell — the nearest
/// flat ground the picture offers.
///
/// Colour comes from [`v6_layout::chrome_run_ink`], the same rule that decides
/// what the RASTER would have drawn, so the glyph is the rasterised glyph made
/// crisp rather than a second opinion about it — and since SQ-1060 it is asked
/// with the same ARGUMENT too, per glyph at its own declared cell. The rule was
/// always shared; the argument was not, and SQ-1052 moved the raster to a
/// per-glyph probe while this side went on asking once per run. A run straddling
/// the edge of the artwork then resolved its block once for its whole length here
/// and per cell there, which is one frame rendering two ways. A run the game gave a real
/// background keeps that background, opaque, exactly as the pixels would; a run
/// with inherited colours gets the sampled ground and sits in the picture.
#[allow(clippy::too_many_arguments)]
fn stamp_runs_over_art(
    runs: &[&crate::engine::PxText],
    art_rects: &[Rect],
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    gfx: &image::RgbaImage,
    default_fg: image::Rgba<u8>,
    default_bg: image::Rgba<u8>,
    colors: &ColorScheme,
    buf: &mut Buffer,
    cell: zvm::screen::V6Cell,
) {
    use std::collections::HashMap;
    let font_w = u32::from(cell.w());
    let font_h = u32::from(cell.h());

    let rgb = |c: image::Rgba<u8>| ratatui::style::Color::Rgb(c.0[0], c.0[1], c.0[2]);
    // SQ-0892's rule, which this path has to obey like every other glyph path: a
    // run is POSITIONED through the scale — `run_cell` on its own origin — and then
    // ADVANCES ONE TERMINAL COLUMN per character. Mapping each character's own
    // native pixel instead looks more principled and is wrong: at the measured
    // Zork Zero frame (120x40 pane, native 640x400, s = 1.875) one 8-px native
    // character is 1.5 columns, so consecutive letters round to every OTHER cell
    // and the label comes out "B▄an▄qu▄et▄ H▄al▄l" with the ribbon showing between
    // its letters. Terminal text is one cell per character whatever the art does.
    let in_art = |col: i32, row: i32| -> bool {
        art_rects.iter().any(|r| {
            col >= r.x as i32 && col < r.right() as i32 && row >= r.y as i32 && row < r.bottom() as i32
        })
    };

    // Every cell about to be stamped, with the ground the ART left in it. Sampled
    // in a pass of its own because a scale below 1 maps two characters into one
    // cell, and sampling as we stamp would read the first glyph's own background
    // back as if it were the picture.
    //
    // Grouped by run because the STYLE bits are the run's; the colour rule is
    // asked per cell (SQ-1060), so each glyph carries the NATIVE x of its own
    // declared cell alongside the pane cell it lands in.
    let mut placed: Vec<(&crate::engine::PxText, StampedCells)> = Vec::new();
    for t in runs {
        let (col0, row) = run_cell(t, scale, cell_px, pane, cell);
        // The cell path places by the DECLARED cell — `col0 + i` above — so glyph
        // `i` occupies native `[px0 + i*font_w, +font_w)`. `pen_chains` does not
        // run here, so a run is what the game published.
        let px0 = t.x.max(1) as u32 - 1;
        let cells: StampedCells = t
            .text
            .chars()
            .enumerate()
            .filter_map(|(i, ch)| {
                let col = col0 + i as i32;
                (col >= 0 && row >= 0 && in_art(col, row))
                    .then(|| (col as u16, row as u16, ch, px0 + i as u32 * font_w))
            })
            .collect();
        if !cells.is_empty() {
            placed.push((t, cells));
        }
    }
    let mut ground: HashMap<(u16, u16), ratatui::style::Color> = HashMap::new();
    for (col, row, _, _) in placed.iter().flat_map(|(_, cells)| cells) {
        let Some(cell) = buf.cell((*col, *row)) else { continue };
        let mean = match (cell.fg, cell.bg) {
            (ratatui::style::Color::Rgb(r0, g0, b0), ratatui::style::Color::Rgb(r1, g1, b1)) => {
                ratatui::style::Color::Rgb(
                    ((u16::from(r0) + u16::from(r1)) / 2) as u8,
                    ((u16::from(g0) + u16::from(g1)) / 2) as u8,
                    ((u16::from(b0) + u16::from(b1)) / 2) as u8,
                )
            }
            // A cell the band did not paint in true colour has no picture in it to
            // match; the run's own resolved background is the honest fallback.
            _ => rgb(default_bg),
        };
        ground.insert((*col, *row), mean);
    }

    for (t, cells) in placed {
        let py = t.y.max(1) as u32 - 1;
        // Reverse is already resolved into the pair by `chrome_run_ink`, so the
        // REVERSED modifier must NOT be set as well — the terminal would swap the
        // pair a second time and paint the picture's own colour as ink on a solid
        // block, which is exactly the block SQ-0487 says must not be there.
        let mut base_style = ratatui::style::Style::default();
        if t.style & 2 != 0 {
            base_style = base_style.add_modifier(ratatui::style::Modifier::BOLD);
        }
        if t.style & 4 != 0 {
            base_style = base_style.add_modifier(ratatui::style::Modifier::ITALIC);
        }
        for (col, row, ch, gx) in cells {
            // **The over-art question is the GLYPH's** (SQ-1060). It was asked once
            // per run, over `chars * font_w` at the run's origin — and
            // `region_has_opaque` answers "is ANY pixel here opaque?", so one stray
            // pixel anywhere beneath a run decided for every cell of it. SQ-1052
            // fixed exactly that on the raster side and left this one, under a doc
            // promising the glyph here is "the rasterised glyph made crisp rather
            // than a second opinion about it". The rule was shared; the ARGUMENT
            // was not, so a run straddling the edge of the frame art resolved its
            // block once for its whole length here and per cell there — same frame,
            // two backends, two ribbons.
            //
            // Not more work in total: a run-wide scan covers the same area this
            // splits into `font_w`-wide pieces, and `chrome_run_ink` only reaches
            // the closure at all on an inherited-colour reverse run.
            let (ink, block) = crate::render::v6_layout::chrome_run_ink(t, default_fg, default_bg, colors, || {
                crate::render::v6_layout::region_has_opaque(gfx, gx, py, font_w, font_h)
            });
            // A run the game gave a real background keeps it, opaque, exactly as
            // the pixels would; otherwise the ground is the art this cell held.
            let bg = block
                .map_or_else(|| ground.get(&(col, row)).copied().unwrap_or(rgb(default_bg)), rgb);
            buf.set_stringn(col, row, ch.to_string(), 1, base_style.fg(rgb(ink)).bg(bg));
        }
    }
}

fn draw_chrome_text_strip(
    runs: &[&crate::engine::PxText],
    rect: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    native: (u16, u16),
    base: ratatui::style::Style,
    ink: TextInk,
    buf: &mut Buffer,
    cell: zvm::screen::V6Cell,
) {
    use crate::engine::PxText;
    use std::collections::BTreeMap;

    // SQ-0508(a)/SQ-0512: fill the WHOLE strip first, so the menu/status panel reads
    // as one solid block — the cells around and between the runs no longer show the
    // theme backdrop. The fill is COLOUR-AWARE: resolve the first run in the strip
    // that set an explicit game colour (per channel) and flood with THAT bg/fg over
    // the themed `base`, so a game (Shogun) that paints its status band with an
    // explicit background floods the whole band, not just the glyph cells, and the
    // blank/bridged gap rows between run rows read as part of the same panel. When no
    // run sets an explicit colour this is byte-identical to a bare `base` flood
    // (Journey's black menu panel, Arthur's status strip). `base` is the
    // `upper_window` theme style; a per-run explicit colour still wins where the run
    // stamps over this fill below.
    let strip_fg = runs.iter().map(|t| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
    let strip_bg = runs.iter().map(|t| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
    let strip_fill = if crate::render::v6_layout::packed_explicit(strip_fg)
        || crate::render::v6_layout::packed_explicit(strip_bg)
    {
        v6_run_style(base, strip_fg, strip_bg, 0, ink)
    } else {
        base
    };
    // …and it stops at the game SCREEN's edge, not the band's (SQ-0946). A band runs
    // to the pane edge, and where the letterbox leaves a horizontal margin the strip
    // was flooding the game's own ground into it — Journey's bottom command strip,
    // nine columns past the frame on each side at a 98x37 pane with `v6_pixel_lock`
    // on. Nothing changes where the art fills the pane, which is every width-bound
    // fit with the lock off.
    let (screen_lo, screen_hi) = crate::render::v6_layout::screen_cols(scale, native.0, cell_px, pane);
    for y in rect.y..rect.bottom() {
        for x in rect.x.max(screen_lo)..rect.right().min(screen_hi) {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(strip_fill);
            }
        }
    }

    // Bucket runs by their GAME text row, laid out on CONSECUTIVE terminal rows
    // from the strip's top (SQ-0543).
    //
    // The chrome ring's ART scales with the pane, but terminal TEXT does not —
    // it is always one terminal cell tall. So the taller the pane, the more
    // terminal rows one 16px game row spans, and at a large pane two adjacent
    // status lines map two rows apart: Shogun's two-line band grew a blank row
    // straight through its middle. Inside a TEXT strip there is no art to stay
    // aligned with — having no frame graphics behind it is what MAKES it a text
    // strip — so the game's own row structure is the truth to preserve, not the
    // device-pixel position.
    //
    // The strip begins at its first text row (`decompose_chrome_strips` carves
    // it that way), so offsetting each run's game row from the topmost one lands
    // the first row exactly where it does today. Genuinely blank game rows
    // survive, since their indices differ by more than one; and wherever the old
    // mapping already produced consecutive rows — any pane small enough that a
    // game row is about a terminal row — the result is byte-identical.
    let font_h = i32::from(cell.h()); // the v6 text cell is 8×16 (SQ-0479)
    let game_row = |t: &PxText| (t.y.max(1) as i32 - 1) / font_h;
    let first_row = runs.iter().map(|t| game_row(t)).min().unwrap_or(0);
    let mut raw: BTreeMap<i32, Vec<&PxText>> = BTreeMap::new();
    for t in runs {
        raw.entry(rect.y as i32 + game_row(t) - first_row).or_default().push(t);
    }
    // SQ-0509: merge horizontally-contiguous same-style fragments before mapping.
    // Runs separated by a genuine gap (Journey's menu items / column dividers,
    // 8px apart) stay distinct and keep their proportional spacing, so the strip
    // bridges only ABUTTING fragments — and never bridges INK to PADDING, which is
    // what let Shogun's two status rows disagree about where native x 503 is
    // (SQ-0757). SQ-0742 first collapses each repeated-glyph RULE to the width of
    // its own scaled span, and flags it, so the stamping below can close the seams
    // around it (see `collapse_row_rules`).
    let mut by_row: BTreeMap<i32, Vec<(PxText, bool)>> = BTreeMap::new();
    for (row, mut rr) in raw {
        rr.sort_by_key(|t| t.x);
        by_row.insert(row, collapse_row_rules(&rr, scale, cell_px, pane, cell));
    }

    // SQ-0892: if this strip is a BLOCK the game composed in its own text grid — one
    // run per row, nothing standing beside anything — place its runs by their offset
    // in native text cells from one shared origin, rather than mapping each one's own
    // native pixel through the scale. See [`strip_native_origin`] for the measurement
    // and for why a row with two runs on it is refused. Asked AFTER the collapse, so
    // the count is of runs as they will be drawn.
    let drawn: Vec<&PxText> = by_row.values().flat_map(|r| r.iter().map(|(t, _)| t)).collect();
    let native_origin = strip_native_origin(&drawn, scale, cell_px, pane, cell);
    let native_x0 = drawn.iter().map(|t| t.x.max(1) as i32 - 1).min().unwrap_or(0);

    // SQ-0508(b): divider columns to draw continuously. A reversed WHITESPACE run is
    // a vertical column divider — a reverse-video space is a solid block, which is how
    // these games draw a rule — and the scale bridges some rows in as blank Text rows,
    // so a column has to be drawn continuously or the line breaks up.
    //
    // **Continuously between the rows the GAME painted it on, not down the whole
    // strip** (SQ-1035). The strip's rect reaches past the window that owns the rule:
    // on Arthur's F3 inventory the two dividers belong to window 2, which ends at the
    // score bar, and running them to `rect.bottom()` drew them straight through the bar
    // and on down the story window — four rows of rule under a bar that
    // `machine-screenshots/amiga-arthur-inventory.png` shows the rules stopping at.
    // Spanning first-painted to last-painted keeps SQ-0508(b)'s bridging (the gap rows
    // are INTERIOR to that span) and stops where the game stopped.
    //
    // Collected from every row except a BAR row, whose edge-to-edge fill below would
    // subsume a rule anyway.
    let mut divider_rows: BTreeMap<u16, (i32, i32)> = BTreeMap::new();
    for (row, row_runs) in &by_row {
        if row_is_reverse_bar(row_runs.iter().map(|(t, _)| t)) {
            continue;
        }
        for (t, _) in row_runs {
            if t.style & 1 != 0 && t.text.trim().is_empty() {
                let (c, _) = run_cell(t, scale, cell_px, pane, cell);
                if c >= rect.x as i32 && c < rect.right() as i32 {
                    let span = divider_rows.entry(c as u16).or_insert((*row, *row));
                    span.0 = span.0.min(*row);
                    span.1 = span.1.max(*row);
                }
            }
        }
    }
    if !divider_rows.is_empty() {
        let rev = v6_run_style(base, 0, 0, 1, ink);
        for (&c, &(first, last)) in &divider_rows {
            let lo = first.max(rect.y as i32);
            let hi = last.min(rect.bottom() as i32 - 1);
            for y in lo..=hi {
                buf.set_stringn(c, y as u16, " ", 1, rev);
            }
        }
    }

    for (row, row_runs) in &by_row {
        if *row < rect.y as i32 || *row >= rect.bottom() as i32 {
            continue;
        }
        // SQ-0512: flood this row's FULL strip width before stamping its runs, so the
        // row reads as one solid panel — the cells around AND between the runs carry
        // the row's own background, not the theme backdrop. The fill colour is the
        // first run in the row with an explicit game colour, per channel, over `base`
        // (Shogun's status band floods its explicit white edge to edge). A PURE
        // reverse-video row (a bar the game draws edge to edge — Arthur's status row,
        // Journey's menu header) floods reversed, subsuming the old pure-reverse gap
        // fill (SQ-0504): the runs re-stamp reversed over it, so a full-width band
        // spans the whole pane. A MIXED row (Journey's menu body — normal verbs among
        // reversed dividers) is NOT flood-reversed; its reversed divider runs re-stamp
        // over an un-reversed flood below. Colourless non-reverse rows keep the strip
        // `base` flood untouched (byte-identical), so Journey's menu body is unchanged.
        let all_rev = row_is_reverse_bar(row_runs.iter().map(|(t, _)| t));
        let row_fg = row_runs.iter().map(|(t, _)| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_bg = row_runs.iter().map(|(t, _)| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        if all_rev
            || crate::render::v6_layout::packed_explicit(row_fg)
            || crate::render::v6_layout::packed_explicit(row_bg)
        {
            let fill = v6_run_style(base, row_fg, row_bg, all_rev as u8, ink);
            // Bounded by the game SCREEN, like the strip flood above (SQ-0946).
            for c in rect.x.max(screen_lo)..rect.right().min(screen_hi) {
                buf.set_stringn(c, *row as u16, " ", 1, fill);
            }
        }
        // SQ-0727: the cells this row's GLYPH runs occupy, so a blank run cannot
        // erase one. A run is POSITIONED by its scale-mapped native pixel, but its
        // characters then advance ONE TERMINAL COLUMN each — two different rates
        // wherever the pane is not exactly one column per native 8px text cell (at
        // 120 columns of a 640px screen it is one and a half). So a blank run the
        // game painted over another run's OWN whitespace no longer lands on that
        // whitespace once mapped: it lands on a neighbouring glyph.
        //
        // advent.z6's help screen is the report. It paints each bar row as one
        // label run plus the reversed blank cells of the bar around and between the
        // labels, and at 120 columns the blanks at native x=17/33/73 mapped onto
        // "N = next subject"'s columns 3/6/14 — its `=` and both lowercase `e`s —
        // while the blank at x=113, one native cell past the label's last character,
        // mapped INSIDE its cell span and clipped the tail off "RETURN = read
        // subjec[t]". Interior drops and a clipped tail, one mechanism. At 80
        // columns the two rates coincide and the row was always correct, which is
        // why this reads as a font bug and is a scale bug.
        //
        // A blank run carries no glyphs: the strip and row floods above already put
        // its background down, and in NATIVE pixels it only ever covers whitespace
        // the glyph run drew itself. So it may still paint the cells no glyph run
        // claimed (the bar's own gaps), and must skip the rest.
        //
        // SQ-0742: a RULE ([`collapse_row_rules`]) closes the seams a scale leaves
        // around it — it runs from the end of whatever is drawn before it to the
        // start of whatever comes after, so a border reads as one unbroken line
        // through its own corners and titles instead of a rule with a hole either
        // side of every neighbour. Everything else keeps exactly the span its
        // characters occupy.
        let base_span = |t: &PxText| {
            // SQ-0892: in a strip with no horizontal structure the column is the
            // run's offset in NATIVE TEXT CELLS from the strip's own origin;
            // everywhere else it is this run's own native pixel through the scale.
            let c = match native_origin {
                Some(o) => o + ((t.x.max(1) as i32 - 1 - native_x0) as f32 / f32::from(cell.w())).round() as i32,
                None => run_cell(t, scale, cell_px, pane, cell).0,
            };
            // SQ-0783: a LONE frame glyph standing at the game screen's own edge — the
            // `┐` that ends the top rule, the `┘` that ends the bottom one, the `│` that
            // closes the menu header — aligns to the far end of its own native cell, so
            // the frame reaches the pane's last column instead of leaving the one beside
            // it blank. Everything else, including every interior divider, keeps
            // `run_cell`'s answer exactly. The rule in front of it runs to whatever
            // column this returns (its right edge IS this span's start), so the line
            // stays unbroken through the corner.
            let c = match t.text.chars().next() {
                Some(g) if t.text.chars().count() == 1 && is_box_glyph(g) => {
                    edge_glyph_col(t.x.max(1) as u32 - 1, native.0 as u32, scale, cell_px, pane, cell).unwrap_or(c)
                }
                _ => c,
            };
            (c, c + t.text.chars().count() as i32)
        };
        let mut spans: Vec<(i32, i32)> = Vec::with_capacity(row_runs.len());
        for (i, (t, rule)) in row_runs.iter().enumerate() {
            let (c0, c1) = base_span(t);
            let span = if *rule {
                // SQ-0747: "where the last thing before it ends" is the end of the last
                // thing DRAWN, which is not always the immediately preceding run.
                //
                // A run is POSITIONED through the scale but advances ONE TERMINAL
                // COLUMN per character, and the two rates only coincide at one column
                // per native 8px cell. So a BLANK run the game paints after a label —
                // over the label's own trailing whitespace, in native pixels — maps to
                // a column INSIDE the label once the label has advanced at the other
                // rate, and ends there. Taking that as the rule's left edge started the
                // rule inside the label and stamped its glyph over the tail: Journey's
                // release-30 menu header came out `The P` at a 115-column pane and
                // `The Pa` at 157, with `Individual Comm` beside it — the eaten labels
                // this quest has carried through five passes, and the reason the count
                // varied with the pane rather than staying put. SQ-0727 fixed the same
                // rate mismatch for the blank run's OWN stamping; this is it one level
                // up, in what the blank's span is then used to bound.
                //
                // So the rule starts no further left than the end of every GLYPH span
                // before it. Monotone: this can only ever move a rule's left edge to
                // the RIGHT of where it is today, and only past ink.
                let prev = spans.last().map_or(c0, |&(_, prev_end)| prev_end);
                let left = row_runs[..i]
                    .iter()
                    .zip(&spans)
                    .filter(|((p, _), _)| !p.text.trim().is_empty())
                    .map(|(_, &(_, e))| e)
                    .max()
                    .map_or(prev, |ink| prev.max(ink));
                let right = row_runs.get(i + 1).map_or(c1, |n| base_span(&n.0).0);
                (left, right.max(left))
            } else {
                (c0, c1)
            };
            spans.push(span);
        }
        // The cells each run's own glyphs occupy, kept alongside the run's index so a
        // run can ask what OTHER runs claim (SQ-0747). `WORD` is a multi-character
        // label — a word the game printed, which nothing single-glyph may overwrite.
        let claimed: Vec<(usize, (i32, i32), bool)> = row_runs
            .iter()
            .zip(&spans)
            .enumerate()
            .filter(|(_, ((t, _), _))| !t.text.trim().is_empty())
            .map(|(i, ((t, _), &s))| (i, s, t.text.chars().count() > 1))
            .collect();
        let is_claimed = |c: i32| claimed.iter().any(|&(_, (lo, hi), _)| c >= lo && c < hi);
        // …and the same question asked by a lone frame glyph: is this cell already a
        // WORD's, drawn by a different run?
        //
        // SQ-0747: Journey's release 30 prints its menu header by drawing the rule
        // FIRST and then printing the title over it, so the row carries both — dozens
        // of `─` fragments and, at overlapping native columns, one run per letter of
        // "The Party". The letters split the rule into groups of one and two, which are
        // too few to be a rule ([`RULE_MIN`]) and so take the DIVIDER path: stamped
        // individually at their own scaled columns. A label advances one terminal
        // column per character while a fragment is positioned through the scale, so
        // those columns land INSIDE the title, and each stray `─` punched a hole in it
        // — `The P─rty`, `Individual Comm─nds`. A divider exists to hold a column the
        // merge would drag off; it has no business overwriting a word.
        let over_word = |i: usize, c: i32| {
            claimed.iter().any(|&(j, (lo, hi), word)| word && j != i && c >= lo && c < hi)
        };
        for (i, ((t, rule), &(col, end))) in row_runs.iter().zip(&spans).enumerate() {
            if col >= rect.right() as i32 || end <= rect.x as i32 {
                continue;
            }
            // SQ-0949: a run that STARTS left of the strip is clipped, not dropped.
            //
            // The strip no longer always spans the pane — a ribbon reaches as far as
            // its own window and the flank keeps the sub-cell remainder beside it
            // (`ChromeRowOracle::blocked`) — so a run positioned through the scale can
            // begin one column outside a strip it is otherwise entirely inside.
            // Arthur's status band is the case: its window opens at native 28, which
            // is column 5.03 at a 115-column pane, and the whole `Churchyard` fragment
            // was thrown away for the sake of the padding cell in front of it.
            let clip = (rect.x as i32 - col).max(0) as usize;
            let col = col + clip as i32;
            let style = v6_run_style(base, t.fg, t.bg, t.style, ink);
            let max_w = rect.right() as usize - col as usize;
            if max_w == 0 {
                continue;
            }
            if let Some(g) = rule.then(|| t.text.chars().next()).flatten() {
                let text: String = std::iter::repeat_n(g, (end - col).max(0) as usize).collect();
                buf.set_stringn(col as u16, *row as u16, &text, max_w, style);
                continue;
            }
            // Untrusted game text (SQ-0639).
            let text: String = crate::render::blank_control_chars(&t.text).chars().skip(clip).collect();
            if t.text.trim().is_empty() {
                for (k, ch) in text.chars().take(max_w).enumerate() {
                    let c = col + k as i32;
                    if !is_claimed(c) {
                        buf.set_stringn(c as u16, *row as u16, ch.encode_utf8(&mut [0u8; 4]), 1, style);
                    }
                }
            } else if text.chars().count() == 1 && over_word(i, col) {
                continue;
            } else {
                buf.set_stringn(col as u16, *row as u16, text.as_str(), max_w, style);
            }
        }
    }
}

/// SQ-0742: collapse each repeated-glyph RULE in one native text row to the width
/// of its own SCALED span, then merge what is left with [`merge_row_fragments`].
///
/// A text strip POSITIONS a run through the letterbox scale but then advances ONE
/// TERMINAL COLUMN per character — the two rates only coincide where the pane is
/// exactly one column per native 8px text cell. For a label that is exactly right:
/// prose has to stay legible, so its character count is what it is. For a RULE it
/// is wrong, because a rule is a *distance* the game drew across, not a string of
/// that many characters. Journey under the Amiga interpreter draws its whole frame
/// that way — `┌`, seventy-eight `─` fragments, `┐` — and one cell per fragment
/// stopped the border at column 79 of a 138-column pane while the prose beside it
/// wrapped to the pane. The same frame under the IBM PC profile is reverse-video
/// SPACES, which the row flood already spreads edge to edge, which is why only the
/// Amiga route ever showed it.
///
/// A rule is [`RULE_MIN`] or more ABUTTING fragments, each a single SYMBOL glyph
/// (never a letter or digit), all at the same style and colours. Each such group
/// becomes one run repeating that glyph across the cells its native span maps to,
/// and is kept OUT of the fragment merge so an adjoining corner or title cannot
/// glue itself on and drag the row back to one cell per native character.
///
/// The predicate is deliberately narrow, because the fragments it reads are the
/// same ones SQ-0509 exists to reassemble: a game with proportional metrics emits
/// one run per GLYPH, so "Anne" arrives as `A` `n` `n` `e` and a rule test of "two
/// abutting equal fragments" reads every doubled letter in the corpus as a rule —
/// Arthur's status bar lost its character's name to exactly that. Requiring a
/// non-alphanumeric glyph and three of them in a row leaves prose alone while
/// still catching every frame rule, which no game draws in two segments.
///
/// Blank runs are left alone as well: the strip and row floods already spread a
/// reverse-video bar edge to edge, so skipping them keeps every game that draws
/// its chrome that way byte-identical.
///
/// SQ-0742 (second pass): a LONE box-drawing or block glyph is likewise kept out
/// of the fragment merge, and stamped at its own scaled column. A rule is a
/// *distance*; a divider is a *position* — and SQ-0509's merge, which exists to
/// re-assemble a WORD, drags one along with whatever abuts it and then advances
/// one terminal cell per character. Journey's command menu abuts each party
/// member's `-->` marker to the `▌` that divides the party column from the
/// commands (native px 246 and 248), so the divider drew three columns left of
/// where it belongs on every row that carries a marker and at its true column on
/// the row that does not — the menu's columns visibly zig-zagged, at 83% of the
/// pane widths swept. Prose has no box glyphs in it, so nothing else moves: the
/// class is exactly the characters a game frames with (U+2500 box drawing,
/// U+2580 block elements), not "punctuation", which Arthur emits one run per
/// glyph of and which must keep merging into its words.
fn collapse_row_rules(
    row_runs: &[&crate::engine::PxText],
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    cell: zvm::screen::V6Cell,
) -> Vec<(crate::engine::PxText, bool)> {
    use crate::engine::PxText;
    /// Fewest abutting fragments that count as a rule rather than as prose.
    const RULE_MIN: usize = 3;
    // Is `t` one glyph the game could have repeated into a rule?
    let single_glyph = |t: &PxText| -> Option<char> {
        let mut cs = t.text.chars();
        match (cs.next(), cs.next()) {
            (Some(c), None) if !c.is_whitespace() && !c.is_alphanumeric() => Some(c),
            _ => None,
        }
    };
    let end_px = |t: &PxText| (t.x.max(1) as i32 - 1) + t.text.chars().count() as i32 * i32::from(cell.w());
    // A glyph from the frame-drawing blocks: box drawing (U+2500..) and block
    // elements (U+2580..). What a game builds chrome geometry out of, and nothing
    // any game's prose contains.
    let box_glyph = |c: char| ('\u{2500}'..='\u{259F}').contains(&c);
    // SQ-0780: …unless the game printed that glyph UNDERNEATH a label, in which case
    // it is not a divider and not a position — it is a leftover of the rule the label
    // was printed over, and the label owns those pixels.
    //
    // Journey's release-30 menu header draws the rule first and the two titles over it,
    // and a stray `─` survives inside each title's native span: `The Party` runs native
    // 152..224 with one at 176, `Individual Commands` runs 368..520 with one at 448.
    // A LABEL is positioned through the scale and then advances one terminal column per
    // character; a fragment is positioned through the scale and stops there. Past about
    // 1.9 columns per native cell the second rate outruns the first, and 80 native
    // pixels into a 19-character title that is a whole column: the stray landed at 110
    // where the title had only reached 109, so it was too far right for SQ-0747's
    // over-a-word guard to suppress, and the rule behind it — which starts no further
    // left than the last thing drawn — began at 111. One blank cell between the title
    // and its rule, at a 159-column terminal and at 83% of the widths swept from there
    // up. `The Party`'s own stray is only 24 native pixels in, still lands inside the
    // title's nine drawn columns at every width swept, and is suppressed — which is
    // exactly why the row's two labels behaved differently and why the asymmetry was
    // the lead.
    //
    // Decided in the game's OWN coordinates, native against native, so the answer
    // cannot move with the pane: mixing a scaled column against a character-advanced
    // span is the ceil-vs-round-on-a-shared-boundary trap this whole area is prone to.
    // The menu body's real dividers are untouched — Journey abuts each `▌` to the END
    // of a `-->` marker (native 246 and 248), never inside one — and SQ-0742's whole
    // point, that a divider holds a column of its own, stands.
    let native_span = |t: &PxText| {
        let x0 = t.x.max(1) as i32 - 1;
        (x0, x0 + t.text.chars().count() as i32 * i32::from(cell.w()))
    };
    let under_label = |t: &PxText| {
        let (a0, a1) = native_span(t);
        row_runs.iter().any(|p| {
            p.text.chars().count() > 1 && !p.text.trim().is_empty() && {
                let (b0, b1) = native_span(p);
                a0 >= b0 && a1 <= b1
            }
        })
    };
    let mut out: Vec<(PxText, bool)> = Vec::new();
    // Pending non-rule fragments, merged together on the far side of each rule so
    // SQ-0509's fragment bridging is unchanged everywhere a rule is not involved.
    let mut pending: Vec<&PxText> = Vec::new();
    let mut i = 0usize;
    while i < row_runs.len() {
        let t = row_runs[i];
        // How far does a run of this same glyph, abutting, continue?
        let mut j = i;
        if let Some(g) = single_glyph(t) {
            while j + 1 < row_runs.len() {
                let n = row_runs[j + 1];
                if single_glyph(n) == Some(g)
                    && (n.x.max(1) as i32 - 1) == end_px(row_runs[j])
                    && n.style == t.style
                    && n.fg == t.fg
                    && n.bg == t.bg
                {
                    j += 1;
                } else {
                    break;
                }
            }
        }
        if j + 1 - i >= RULE_MIN {
            // A rule. Flush what came before it, then emit it at its scaled width.
            out.extend(merge_strip_fragments(&pending).into_iter().map(|t| (t, false)));
            pending.clear();
            let (col0, _) = run_cell(t, scale, cell_px, pane, cell);
            let cw = cell_px.0.max(1) as f32;
            let end_dev = scale.off_x as f32 + end_px(row_runs[j]) as f32 * scale.s;
            let col1 = pane.x as i32 + (end_dev / cw).round() as i32;
            let cells = (col1 - col0).max(1) as usize;
            let mut rule = t.clone();
            rule.text = std::iter::repeat_n(single_glyph(t).expect("checked above"), cells).collect();
            out.push((rule, true));
            i = j + 1;
            continue;
        }
        // Too few for a rule, but still frame geometry: a DIVIDER. Flush what came
        // before it and stamp each fragment on its own, so the merge cannot drag it
        // off the column the game drew it at.
        if single_glyph(t).is_some_and(box_glyph) {
            out.extend(merge_strip_fragments(&pending).into_iter().map(|t| (t, false)));
            pending.clear();
            out.extend(row_runs[i..=j].iter().filter(|t| !under_label(t)).map(|t| ((*t).clone(), false)));
            i = j + 1;
            continue;
        }
        pending.push(t);
        i += 1;
    }
    out.extend(merge_strip_fragments(&pending).into_iter().map(|t| (t, false)));
    out
}

/// SQ-0757: merge a text strip's fragments the way [`merge_row_fragments`] does,
/// but never glue a field to the PADDING in front of it.
///
/// The merge exists to re-assemble text the game printed as one stream and
/// proportional pixel metrics scattered into one run per glyph. A merged run is
/// POSITIONED once through the letterbox scale and then advances ONE TERMINAL COLUMN
/// per character, and those two rates coincide only where a terminal column is
/// exactly one native 8px text cell. Inside a phrase that is the point — prose has
/// to stay legible, so its character count is what it is, and Arthur's
/// "St Anne's Day, Compline" has to arrive as that and not as four words at four
/// independently rounded columns. Across the PADDING between two FIELDS it is simply
/// wrong: gluing them hands the second field the first one's starting column plus a
/// native-width advance, so it lands correctly at the one pane width where the rates
/// agree and drifts everywhere else, by an amount that grows with how much padding
/// it was glued to.
///
/// The line between the two is the width of the blank stretch. One blank cell is a
/// word space — part of what the game printed. [`FIELD_GAP_PX`] or more is layout,
/// and what lies past it is a field with a column of its own.
///
/// Shogun off its Amiga release floppy is the report. Under interpreter 4 the game
/// paints its status band one run per CELL, padding included, so the strip received
/// row 0 as `Erasmus` `:` + 23 blanks + `SHOGUN` + 21 blanks + `Score:` … and row 1
/// as `Bridge` + 51 blanks + `Moves:` … — each glued into one run, and the two rows
/// began at different native x. Both put their right-hand field at native x 503, and
/// the glue placed them in DIFFERENT columns at every pane width except the one where
/// a terminal column is a native cell: at an 80-column story pane (82–83 terminal
/// columns once the frame's borders are counted) `Score:` and `Moves:` lined up, and
/// nowhere else did they. The same game under the IBM PC profile emits one run per
/// FIELD with no padding runs at all, so nothing was ever glued and it right-justifies
/// at every width — the report's own control, and the reason this reads as an Amiga
/// defect when it is a padding defect.
///
/// Padding still merges with itself, so a row of it is one run rather than fifty; and
/// a blank run only ever paints the cells no glyph run claimed (SQ-0727), so wherever
/// it lands it cannot rub a field out.
fn merge_strip_fragments(row_runs: &[&crate::engine::PxText]) -> Vec<crate::engine::PxText> {
    use crate::engine::PxText;
    /// Blank CELLS that separate two FIELDS rather than two words: more than the
    /// single cell a game puts between words when it prints them. Counted in cells
    /// since SQ-1009 — it was `8 * chars` native pixels, which is neither the
    /// Macintosh's 7-wide cell nor any proportional pen.
    const FIELD_GAP_CELLS: u16 = 1;
    let ink = |t: &PxText| !t.text.trim().is_empty();
    let width = |t: &PxText| t.text.chars().count() as u16;
    let mut out: Vec<PxText> = Vec::new();
    let mut group: Vec<&PxText> = Vec::new();
    // Blank fragments since the last inked one, held back until the next inked one
    // says whether they were a word space or a field gap.
    let (mut blanks, mut blank_px): (Vec<&PxText>, u16) = (Vec::new(), 0);
    for t in row_runs {
        if !ink(t) {
            blanks.push(t);
            blank_px = blank_px.saturating_add(width(t));
            continue;
        }
        if blank_px > FIELD_GAP_CELLS {
            out.extend(merge_row_fragments(&group, 1));
            group.clear();
            out.extend(merge_row_fragments(&blanks, 1));
        } else {
            group.extend(blanks.iter().copied());
        }
        blanks.clear();
        blank_px = 0;
        group.push(t);
    }
    // SQ-0900: the blanks still held at the end of the row get the SAME decision an
    // interior blank gets, rather than being flushed on their own.
    //
    // A blank is held back because only the next inked run can say whether it was a
    // word space or a field gap. A TRAILING blank never gets that run, and emitting
    // it separately positions it by its own native x through the ring's scale while
    // the group beside it advances one terminal column per character — two rates that
    // coincide only where a terminal cell is one native 8px cell (SQ-0892). Shogun's
    // boot menu is the measured case: `"START the game "` arrives as sixteen
    // single-character runs at native x 235..347, all `style=0b0001` reverse video
    // with identical colours and every one abutting, so the fifteen up to `"game"`
    // merge and the final space does not. At a 129x60 pane the group runs to column
    // 62 and the loose space lands at column **70** — a lone reverse-video block
    // stranded eight columns past the item, with the gap widening as the pane grows.
    // Below scale 1 the same run produced the opposite symptom, landing ON the
    // group's last cell and erasing the `e` of "game" (SQ-0898, fixed separately by
    // forbidding a blank to erase a glyph — that fix addresses the COLLISION and this
    // one the PLACEMENT, and both are wanted).
    //
    // Not suppressed, merged: the space keeps painting, still reverse video, still one
    // cell wide, because Shogun's selection bar IS made of reverse-video blanks and
    // they are real ink (SQ-0499/SQ-0515). A trailing blank WIDER than one cell is
    // still a field gap and still keeps its own position, exactly as before.
    if blank_px > FIELD_GAP_CELLS {
        out.extend(merge_row_fragments(&group, 1));
        out.extend(merge_row_fragments(&blanks, 1));
    } else {
        group.extend(blanks.iter().copied());
        out.extend(merge_row_fragments(&group, 1));
    }
    out
}

/// SQ-0509: merge horizontally-contiguous same-style fragments of ONE native text
/// row (`row_runs` sorted by `x`) into single runs. A game that positions status
/// text with proportional pixel metrics — Arthur — emits word fragments as
/// separate runs whose pixel start abuts the previous run's pixel end; placing
/// each fragment independently scatters them ("Chu rch yard", or one anchor group
/// per glyph). A run starting within `tol_px` of the previous run's end, with
/// identical style and colours, is concatenated onto it, the intervening pixel gap
/// becoming `gap / 8` spaces — so a `tol_px` of 4 bridges only ABUTTING fragments
/// (adding nothing) while 8 also closes a one-cell word gap with a real space.
/// Runs separated by a wider gap stay distinct and keep their own positions.
fn merge_row_fragments(row_runs: &[&crate::engine::PxText], tol_cells: i32) -> Vec<crate::engine::PxText> {
    let mut merged: Vec<crate::engine::PxText> = Vec::new();
    for t in row_runs {
        if let Some(last) = merged.last_mut() {
            // In CELLS, from the grid the engine wrote these runs into (SQ-1009).
            // Measured in pixels this asked `(t.x - last.x) / cell.w`, which is the
            // gap between two runs only while the pen advances one declared cell
            // per character — Arthur's does not, so consecutive GLYPHS of one word
            // read as separated by a fraction of a cell and the merge fell apart
            // mid-word: `Churchyard` came back as `Ch urc  hy ard`. A cell gap is
            // exact on both kinds of machine.
            let last_end = i32::from(last.gcol) + last.text.chars().count() as i32;
            let start = i32::from(t.gcol);
            if start >= last_end
                && start - last_end <= tol_cells
                && last.style == t.style
                && last.fg == t.fg
                && last.bg == t.bg
            {
                for _ in 0..(start - last_end) {
                    last.text.push(' ');
                }
                last.text.push_str(&t.text);
                continue;
            }
        }
        merged.push((*t).clone());
    }
    merged
}

/// Render the v6 cell-path status band as a classic full-width status line
/// ("anchored bar", SQ-0467). `runs` are all the chrome grids' pixel-text runs;
/// `ncols` is the native screen width in cells (so anchor thresholds scale to the
/// game's own screen, not a hardcoded 40). Each native row (`(y-1)/16`) below
/// `band_rows` is classified into LEFT/CENTER/RIGHT anchor groups and painted
/// across the full pane width. Returns the number of band rows used (for the
/// story offset).
///
/// `band_rows` is the story window's TOP native row, not a constant (SQ-0549):
/// the status band is whatever chrome text sits ABOVE the story, wherever the
/// game put it. The band is ANCHORED to the pane top — its first inked native row
/// draws at `area.y`, and the rows below keep their relative spacing — so Arthur's
/// row-12 bar (its story buffer starts at row 13, under a 12-row art panel this
/// path drops) reads as a top status line instead of floating a quarter
/// of the way down the pane.
/// How many pane rows [`draw_anchored_status_band`] will use, without painting
/// anything (SQ-0712). The band has to be measured before the story area can be
/// sized and painted after the erase fills, so the two are split: this is the
/// span from the first inked native row inside the band to the last, clamped to
/// the pane, which is exactly what the draw returns.
fn anchored_band_rows(runs: &[&crate::engine::PxText], band_rows: u16, pane_h: u16) -> u16 {
    let inked = || {
        // The rows `draw_anchored_status_band` will DRAW on — the runs' own cell
        // rows (SQ-1009). Measured rows and drawn rows have to be one answer.
        runs.iter()
            .filter(|t| !t.text.trim().is_empty())
            .map(|t| t.grow)
            .filter(|&r| r < band_rows)
    };
    let Some(first) = inked().min() else { return 0 };
    // The draw stops at the pane's bottom edge, so a row that would land off-pane
    // never paints and never counts — the band must stay in-pane either way.
    inked()
        .filter(|&r| r - first < pane_h)
        .max()
        .map(|last| last - first + 1)
        .unwrap_or(0)
}

fn draw_anchored_status_band(
    runs: &[&crate::engine::PxText],
    ncols: u32,
    band_rows: u16,
    area: Rect,
    buf: &mut Buffer,
    style: ratatui::style::Style,
    ink: TextInk,
) -> u16 {
    let left_bound = ncols / 3; // left-third boundary (cells)
    let right_bound = ncols * 2 / 3; // right two-thirds boundary (cells)
    // The band's own origin: the topmost inked native row inside it.
    let Some(first_row) = runs
        .iter()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| t.grow)
        .filter(|&r| r < band_rows)
        .min()
    else {
        return 0;
    };
    let mut rows_used = 0u16;
    for row in first_row..band_rows {
        if area.y + (row - first_row) >= area.bottom() {
            break; // the band must stay in-pane
        }
        // This native row's non-blank runs, across ALL chrome grids, left→right.
        let mut row_runs: Vec<&crate::engine::PxText> = runs
            .iter()
            .copied()
            .filter(|t| !t.text.trim().is_empty() && t.grow == row)
            .collect();
        if row_runs.is_empty() {
            continue;
        }
        row_runs.sort_by_key(|t| t.gcol);
        // Glue the row's word fragments back together before classifying (SQ-0509,
        // reached here by SQ-0549): Arthur paints its bar one GLYPH per run, which
        // would otherwise put every letter in its own anchor group and join them
        // with two spaces apiece. The one-cell tolerance also restores the
        // single-cell word gaps inside its date field ("St Anne's Day, Compline"),
        // which a group join would likewise have doubled.
        let row_runs = merge_row_fragments(&row_runs, 1);
        // Classify each run into an anchor group by its native position. A run
        // the game CENTRED on its own screen is CENTER wherever it starts (see
        // below); otherwise a run spanning most of the row (a full-width bar)
        // counts LEFT, a start in the left third is LEFT, an end past the right
        // two-thirds is RIGHT, and everything between is CENTER. Within a group,
        // run order is preserved and the native gaps collapse to a two-space join.
        let (mut left, mut center, mut right): (Vec<&str>, Vec<&str>, Vec<&str>) =
            (Vec::new(), Vec::new(), Vec::new());
        for t in &row_runs {
            let start = u32::from(t.gcol);
            let len = t.text.chars().count() as u32;
            let end = start + len;
            // SQ-0717: the thirds rule reads a run's START, which is the right
            // question for a status FIELD (a location name begins at the left
            // margin, a score ends at the right one) and the wrong one for a line
            // the game centred by cursor arithmetic. Shogun's frozen title header
            // is nine such lines (SQ-0697) — the longer ones begin left of the
            // left-third boundary and the shortest ends past the right two-thirds,
            // so five of nine were flushed to col 0 and one flushed right, wrecking
            // the block the game had carefully centred. A run with equal margins on
            // its own screen — within the one cell that 8px column quantization can
            // cost — was centred deliberately, so it is CENTER, and the pane centres
            // it again at whatever width the terminal happens to be. Both margins
            // must be non-zero: a rule or bar drawn from the screen edge is not
            // centred text, and stays the LEFT-anchored bar it was.
            let centred =
                start > 0 && end < ncols && start.abs_diff(ncols - end) <= 1;
            if centred {
                center.push(&t.text);
            } else if len * 3 >= ncols * 2 || start < left_bound {
                left.push(&t.text);
            } else if end > right_bound {
                right.push(&t.text);
            } else {
                center.push(&t.text);
            }
        }
        let left_str = left.join("  ");
        let center_str = center.join("  ");
        let right_str = right.join("  ");
        // Resolve this band row's style from its runs (SQ-0488): the first run
        // that set an explicit game colour contributes that channel over the
        // themed base, and any reversed run flips the row — so Zork0's dark-on-
        // tan ribbon labels keep their colours while Shogun's Default/Default
        // runs stay exactly the theme style. The band collapses multiple runs
        // into left/center/right strings painted with one style, so per-channel
        // is first-explicit-wins across the row rather than per-substring.
        let row_fg = row_runs.iter().map(|t| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_bg = row_runs.iter().map(|t| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_rev = row_runs.iter().any(|t| t.style & 1 != 0) as u8;
        let row_style = v6_run_style(style, row_fg, row_bg, row_rev, ink);
        if place_anchored_row(buf, area, area.y + (row - first_row), &left_str, &center_str, &right_str, row_style) {
            rows_used = rows_used.max(row - first_row + 1);
        }
    }
    rows_used
}

/// Paint one anchored status row across the full pane width: LEFT flush at col 0,
/// RIGHT flush to the last column, CENTER centered. Overlap priority (narrow
/// panes): LEFT wins; RIGHT truncates from its left edge to keep ≥1 space from
/// LEFT; CENTER drops entirely if it can't fit between them with a space each
/// side. Never overwrites one group with another; never panics on width 1–2.
/// Returns whether anything was painted.
fn place_anchored_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    left: &str,
    center: &str,
    right: &str,
    style: ratatui::style::Style,
) -> bool {
    let w = area.width as usize;
    if w == 0 {
        return false;
    }
    // Untrusted game text (SQ-0639): blank control chars before any of it reaches
    // the buffer. Blanking is char-for-char, so every width/anchor computation
    // below is unchanged.
    let (left_txt, center_txt, right_txt) = (
        crate::render::blank_control_chars(left),
        crate::render::blank_control_chars(center),
        crate::render::blank_control_chars(right),
    );
    let (left, center, right) = (left_txt.as_ref(), center_txt.as_ref(), right_txt.as_ref());
    let mut painted = false;

    // Fill the WHOLE band row with the status style's background first, so the
    // band reads as one solid bar (the upper_window bg fills the gaps between
    // the anchored groups), not just coloured cells behind the glyphs (SQ-0467
    // follow-up: fill first, stamp runs after).
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }

    // LEFT — flush at col 0, truncated to the pane width.
    let left_len = left.chars().count().min(w);
    if left_len > 0 {
        buf.set_stringn(area.x, y, left, w, style);
        painted = true;
    }

    // RIGHT — flush to the last column; truncate leading chars if it would collide
    // with LEFT (keeping ≥1 space between). Truncating from the left keeps the end
    // flush right.
    let min_right_start = if left_len > 0 { left_len + 1 } else { 0 };
    let mut right_str: String = right.to_string();
    let mut right_len = right_str.chars().count();
    if right_len > 0 {
        let avail = w.saturating_sub(min_right_start);
        if right_len > avail {
            let drop = right_len - avail;
            right_str = right_str.chars().skip(drop).collect();
            right_len = right_str.chars().count();
        }
        if right_len > 0 {
            let right_start = w - right_len;
            buf.set_stringn(area.x + right_start as u16, y, &right_str, right_len, style);
            painted = true;
        }
    }

    // CENTER — centered, but only if it fits in the gap between LEFT and RIGHT with
    // a space on each side; otherwise dropped entirely.
    let center_len = center.chars().count();
    if center_len > 0 {
        let gap_lo = if left_len > 0 { left_len + 1 } else { 0 };
        let gap_hi = if right_len > 0 { (w - right_len).saturating_sub(1) } else { w };
        if gap_hi > gap_lo && center_len <= gap_hi - gap_lo {
            let natural = w.saturating_sub(center_len) / 2;
            let start = natural.clamp(gap_lo, gap_hi - center_len);
            buf.set_stringn(area.x + start as u16, y, center, center_len, style);
            painted = true;
        }
    }

    painted
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// **The over-art question is the GLYPH's on the CELL path too** (SQ-1060).
    ///
    /// `region_has_opaque` answers "is ANY pixel in this rectangle opaque?", and
    /// `stamp_runs_over_art` asked it once per run over `chars * font_w`. One
    /// stray opaque pixel anywhere beneath a run therefore decided SQ-0487's
    /// no-block arm for every cell of it. SQ-1052 fixed exactly that on the raster
    /// side and left this one, under a doc promising the glyph here is "the
    /// rasterised glyph made crisp rather than a second opinion about it" — the
    /// RULE was shared, the ARGUMENT was not.
    ///
    /// Half-blocks only, because [`backend_layers_glyphs_over_art`] admits nothing
    /// else — which is why this is a unit case on a canvas of its own rather than a
    /// real-media one. The frame: one inherited-reverse run of six characters over
    /// artwork opaque under exactly one of them.
    ///
    /// FALSIFY by hoisting the probe back out of the cell loop and asking it once
    /// at `px0` over `chars * font_w`: the patch condemns the whole run and every
    /// cell loses its block.
    #[test]
    fn a_chrome_run_over_art_resolves_its_block_per_cell_on_the_cell_path() {
        let cell = zvm::screen::V6Cell::DEFAULT;
        let (fw, fh) = (u32::from(cell.w()), u32::from(cell.h()));
        // Artwork opaque under the FOURTH glyph alone.
        const OVER: usize = 3;
        let mut gfx = image::RgbaImage::new(fw * 8, fh);
        for y in 0..fh {
            for x in (OVER as u32 * fw)..((OVER as u32 + 1) * fw) {
                gfx.put_pixel(x, y, image::Rgba([9, 9, 9, 255]));
            }
        }
        let run = crate::engine::PxText {
            y: 1,
            x: 1,
            text: "ABCDEF".to_string(),
            style: 1, // reverse, inherited colours — SQ-0487's arm
            fg: 0,
            bg: 0,
            grow: 0,
            gcol: 0,
        };
        // One pane cell per native cell, no letterboxing, so a glyph's column IS
        // its index and the geometry cannot confuse the reading.
        let scale = crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 };
        let pane = Rect::new(0, 0, 8, 1);
        let art = [pane];
        let fg = image::Rgba([220, 220, 220, 255]);
        let bg = image::Rgba([0, 0, 0, 255]);
        let mut buf = Buffer::empty(pane);
        super::stamp_runs_over_art(
            &[&run], &art, &scale, (cell.w(), cell.h()), pane, &gfx, fg, bg,
            &ColorScheme::terminal_default(), &mut buf, cell,
        );

        let block = ratatui::style::Color::Rgb(220, 220, 220);
        let seen: Vec<(char, bool)> = (0..6u16)
            .map(|c| {
                let cell = buf.cell((c, 0)).expect("in the pane");
                (cell.symbol().chars().next().unwrap_or(' '), cell.bg == block)
            })
            .collect();
        assert!(
            seen.iter().enumerate().all(|(i, (ch, _))| *ch == b"ABCDEF"[i] as char),
            "the run was stamped: {seen:?}",
        );
        for (i, (ch, blocked)) in seen.iter().enumerate() {
            if i == OVER {
                assert!(!blocked, "{ch:?} sits ON the artwork and keeps ink without a block");
            } else {
                assert!(
                    blocked,
                    "{ch:?} is clear of the artwork and keeps its reversed block — one opaque \
                     patch under a neighbour must not speak for it: {seen:?}",
                );
            }
        }
    }

    /// **One truncating device→native inverse in this file, not several**
    /// (SQ-1059).
    ///
    /// `ChromeRowOracle::region_has_art`'s doc has always said it is "shared with
    /// the caller's art test so the two do not drift", and it was not: SQ-0894
    /// pasted the body into the oracle rather than calling it, in a commit whose
    /// own message said "the fix should not add a fourth instance of it". The two
    /// copies never diverged, so nothing was mis-rendered — SQ-1020 is what
    /// happens in this same file when a pair like that finally does.
    ///
    /// A de-duplication has no behaviour to falsify, so what is pinned is the
    /// property: the inverse is written ONCE. `inv_x` is its signature — the
    /// column→native mapping with the `as u32` truncation where the rounding bugs
    /// live. `over_art` is deliberately not counted: it asks in exact native
    /// coordinates and never builds this inverse at all.
    #[test]
    fn the_device_to_native_inverse_is_written_once() {
        let src = include_str!("screen.rs");
        let needle = format!("let inv_x = |c: {}|", "u16");
        assert_eq!(
            src.matches(needle.as_str()).count(),
            1,
            "the truncating device→native inverse must exist once in render/screen.rs — a \
             second copy is what SQ-1059 removed, and what SQ-0894 added while promising \
             it had not",
        );
    }


    /// **No native-pixel arithmetic in this file may spell the Version 6 cell as a
    /// number** (SQ-1020).
    ///
    /// This is the second time the same defect shipped from this file, and both
    /// times a grep could not see it. SQ-0917 gave the Macintosh a 7x15 cell and
    /// converted every named `FONT_W`/`FONT_H`; thirty-five sites quantized by a
    /// bare `8` or `16` and were missed, because a constant's NAME is greppable and
    /// a number is not. Its follow-up then converted the divisions and wrote down
    /// what it was leaving: "a THRESHOLD compared against a cell dimension is not
    /// arithmetic and is not fixed by any of this". Six thresholds survived, and
    /// one of them rasterised the Macintosh score bar.
    ///
    /// So this checks the SPELLING, which is the only thing the compiler and the
    /// gate cannot. A native-pixel name added to a cell-sized constant is what both
    /// rounds looked like:
    ///
    /// ```text
    /// py + 16 <= story_top          story_bottom + 16          gnx0 + 8
    /// ```
    ///
    /// Every one of those is `cell.h` or `cell.w` — see [`zvm::screen::V6Cell`],
    /// which now names the extent operations as well as the divisions. If a
    /// genuinely non-cell use of one of these names arises, rename the variable:
    /// the point is that a reader can tell the two apart, and today they cannot.
    #[test]
    fn no_bare_v6_cell_literals_in_native_pixel_arithmetic() {
        // The names that carry NATIVE pixels here. Deliberately the ones the two
        // rounds actually used rather than everything plausible — a discipline case
        // that cries wolf gets deleted, and then it guards nothing.
        const NATIVE: &[&str] =
            &["py", "y_px", "px0", "gnx0", "gnx1", "story_top", "story_bottom", "native_h"];
        // A cell has been 8x16 and 7x15; both axes of both are worth catching.
        const CELLISH: &[&str] = &["7", "8", "15", "16"];

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/render/screen.rs");
        let src = std::fs::read_to_string(&path).expect("this file is readable");
        let mut bad = Vec::new();
        for (n, line) in src.lines().enumerate() {
            // Comments quote the old form on purpose — this case is about code.
            let code = match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            };
            for name in NATIVE {
                for lit in CELLISH {
                    for op in [" + ", " - "] {
                        let needle = format!("{name}{op}{lit}");
                        let Some(at) = code.find(&needle) else { continue };
                        // `apy + 16` is not `py + 16`, and `py + 160` is not either.
                        let before_ok = at == 0
                            || !code.as_bytes()[at - 1].is_ascii_alphanumeric()
                                && code.as_bytes()[at - 1] != b'_';
                        let after = at + needle.len();
                        let after_ok = after >= code.len()
                            || !code.as_bytes()[after].is_ascii_alphanumeric();
                        if before_ok && after_ok {
                            bad.push(format!("  {}:{}  {}", path.display(), n + 1, line.trim()));
                        }
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "native-pixel arithmetic spelling the v6 cell as a literal — use              `cell.w`/`cell.h`, or `V6Cell::rows_px`/`bottom_px` for an extent:\n{}",
            bad.join("\n"),
        );
    }


    // ── merge_strip_fragments, trailing blanks (SQ-0900) ─────────────────────

    /// One reverse-video character run at native `x`, the shape Shogun's boot menu
    /// emits: `style = 1`, default colours, one 8px cell wide.
    fn rv(x: u16, s: &str) -> crate::engine::PxText {
        crate::engine::PxText::derived(337, x, s.into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT)
    }

    /// A trailing blank that ABUTS its group joins it, so the group's own advance
    /// places it (SQ-0900).
    ///
    /// A blank is held back because only the next INKED run can say whether it was a
    /// word space or a field gap. A trailing blank never gets that run, and the old
    /// flush emitted it alone — positioned by its own native x through the ring's
    /// scale, while the group beside it advances one terminal column per character.
    /// The two rates coincide only where a terminal cell is one native 8px cell, so
    /// the stranded space drifted further out the wider the pane got: MEASURED on
    /// `stories/shogun-r322-s890706.z6` at a 129x60 pane, `"START the game "` arrives
    /// as sixteen abutting single-character runs at native x 235..347 and the loose
    /// space landed at column 70 against the group's last column 62.
    ///
    /// Merged, not suppressed — the space is still in the text, still reverse video,
    /// because the selection bar IS reverse-video blanks (SQ-0499/SQ-0515).
    #[test]
    fn a_trailing_blank_that_abuts_its_group_is_placed_by_the_group() {
        let runs: Vec<crate::engine::PxText> = "START the game "
            .chars()
            .enumerate()
            .map(|(i, c)| rv(235 + 8 * i as u16, &c.to_string()))
            .collect();
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let merged = merge_strip_fragments(&refs);
        assert_eq!(
            merged.iter().map(|t| (t.x, t.text.clone())).collect::<Vec<_>>(),
            vec![(235, "START the game ".to_string())],
            "the whole item is ONE run at the group's origin — a trailing space emitted \
             separately is positioned by the scale and strands past the item"
        );
        assert_eq!(merged[0].style, 1, "and it is still reverse video");
    }

    /// A trailing blank WIDER than one cell is a field gap and keeps its own
    /// position, which is the distinction the held-back blanks exist to make.
    ///
    /// Pinned beside the case above because the fix is a decision about these blanks,
    /// and a fix that merged every trailing blank regardless of width would pass that
    /// case and quietly fold a right-anchored field onto the left one.
    #[test]
    fn a_wide_trailing_blank_is_a_field_gap_and_keeps_its_place() {
        let runs = [rv(235, "SCORE"), rv(275, "    ")];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let merged = merge_strip_fragments(&refs);
        assert_eq!(
            merged.iter().map(|t| (t.x, t.text.clone())).collect::<Vec<_>>(),
            vec![(235, "SCORE".to_string()), (275, "    ".to_string())],
            "four blank cells past the group is a field gap, not a word space"
        );
    }

    /// An INTERIOR blank was always merged, and still is — this is the behaviour the
    /// trailing case is being made consistent with, so it is pinned rather than
    /// assumed.
    #[test]
    fn an_interior_word_space_still_merges() {
        let runs = [rv(235, "the"), rv(259, " "), rv(267, "game")];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        assert_eq!(
            merge_strip_fragments(&refs).iter().map(|t| (t.x, t.text.clone())).collect::<Vec<_>>(),
            vec![(235, "the game".to_string())],
        );
    }

    use super::*;
    use crate::engine::{GridWindow, Split};
    use crate::state::StyleRun;
    use ratatui::layout::Rect;

    /// SQ-0828: a flank panel's cell box does not distort the picture in it.
    ///
    /// The defect, exactly: `menu_flank_panel` ceiled `cols` and `rows` INDEPENDENTLY,
    /// and a cell is 8 wide against 18 tall, so the two ceilings rounded by quite
    /// different amounts. Journey's 222x254 plate at an 80x24 pane (uniform scale 1.0)
    /// went into 224x270 — x1.0090 against y1.0630 — and the picture was stretched 5.3%
    /// vertically. Nobody chose that; it fell out of the arithmetic.
    ///
    /// Some quantization is unavoidable on a 8x18 grid, so the property asserted is that
    /// NO whole-cell box within a cell of the ideal distorts less — the function's actual
    /// promise, which cannot be too tight or too slack — plus the magnitude on the one
    /// case the quest reports. Story-free, so it gates in CI where the floppy is absent.
    ///
    /// FALSIFY by restoring the two independent ceilings — `(dw / cw).ceil()` for both
    /// bounds of each axis in `aspect_cells`: the sweep fails on its first shape
    /// ("222x254 at scale 0.5 … landed in a 14x8 cell box = 112x144, stretching the
    /// picture by 12.37% — but 104x126 would have stretched it 5.89%"), and the reported
    /// 222x254 @ s=1.0 case comes back 224x270, the quest's 5.35%.
    #[test]
    fn a_flank_panels_cell_box_keeps_the_pictures_aspect() {
        // How much taller the box is drawn than wide, relative to the art — the quest's
        // own reading of the defect (x1.0090 against y1.0630).
        let stretch = |bw: f32, bh: f32, dw: f32, dh: f32| (bh / dh) / (bw / dw) - 1.0;
        // Journey's plate, then other shapes so the rule is not fitted to one.
        for (aw, ah) in [(222.0f32, 254.0f32), (248.0, 272.0), (111.0, 127.0), (320.0, 200.0)] {
            for s in [0.5f32, 0.725, 1.0, 1.215, 1.6, 1.845, 2.475, 3.69] {
                let (dw, dh) = (aw * s, ah * s);
                let (cols, rows) = aspect_cells(dw, dh, 8, 18, 500, 500);
                let (bw, bh) = (cols as f32 * 8.0, rows as f32 * 18.0);
                let got = stretch(bw, bh, dw, dh).abs();
                // …the box stays within a cell of the ideal on each axis, so this can
                // neither inflate the art to fill its column nor starve it.
                assert!(
                    (bw - dw).abs() <= 8.0 && (bh - dh).abs() <= 18.0,
                    "{aw}x{ah} at scale {s}: {bw}x{bh} is more than one cell from the \
                     ideal {dw}x{dh}"
                );
                // …and it is the least distorting box that is.
                for c in [(dw / 8.0).floor(), (dw / 8.0).ceil()] {
                    for r in [(dh / 18.0).floor(), (dh / 18.0).ceil()] {
                        let (ow, oh) = (c.max(1.0) * 8.0, r.max(1.0) * 18.0);
                        assert!(
                            got <= stretch(ow, oh, dw, dh).abs() + 1e-6,
                            "{aw}x{ah} at scale {s} (ideal {dw}x{dh} device px) landed in \
                             a {cols}x{rows} cell box = {bw}x{bh}, stretching the picture \
                             by {:.2}% — but {ow}x{oh} would have stretched it {:.2}%",
                            got * 100.0,
                            stretch(ow, oh, dw, dh).abs() * 100.0
                        );
                    }
                }
            }
        }
        // The reported case, by the numbers: Journey's plate at an 80x24 pane, where the
        // uniform scale is exactly 1.0 and the art therefore wants its own 222x254.
        let (cols, rows) = aspect_cells(222.0, 254.0, 8, 18, 500, 500);
        assert_eq!((cols, rows), (28, 14), "Journey's plate at s=1.0 goes into 224x252");
        assert!(
            stretch(224.0, 252.0, 222.0, 254.0).abs() < 0.02,
            "the reported 5.35% stretch (224x270) must be under 2%"
        );
    }

    /// The caller's caps still bind — a flank narrowed to leave a column of panel fill
    /// beside the divider gets the best partner for the width it is left with, not a box
    /// that overruns the rule.
    #[test]
    fn a_capped_flank_panel_stays_inside_its_caps() {
        for cap in 1u16..=40 {
            let (cols, rows) = aspect_cells(222.0, 254.0, 8, 18, cap, 12);
            assert!((1..=cap).contains(&cols), "cap {cap}: {cols} columns");
            assert!((1..=12).contains(&rows), "cap {cap}: {rows} rows");
        }
    }

    /// SQ-0894: a side column is as wide as the art is where it is NARROWEST, so a
    /// banner or a capital above it never widens the answer.
    ///
    /// This is the load-bearing decision in [`flank_art_columns`] and the reason the
    /// caller can safely take the wider of it and the story box's leftover: the
    /// statistic can only under-claim, never over-claim, and over-claiming is what
    /// would swallow the screen. Story-free, so it holds in CI where the gitignored
    /// fixtures are absent and every real-game measurement skips.
    ///
    /// The three shapes are the three the corpus actually has, each reduced to its
    /// geometry (measured native columns in the doc comment on the function):
    ///
    /// - **Shogun** — one slab at each edge, transparent between, every row the same.
    /// - **Zork Zero** — pillars, under a banner spanning the WHOLE screen width. The
    ///   banner rows offer the full half-canvas; the pillar rows offer 72.
    /// - **Arthur** — poles, beside a header that starts exactly where the pole ends,
    ///   with no gap between them. The header rows therefore offer one contiguous run
    ///   from the pole straight across the screen, and only the pole-only rows say 28.
    ///   No gutter is needed for the rule to work, which is the point of taking a
    ///   minimum rather than looking for a break in the ink.
    ///
    /// FALSIFY by taking the maximum, or the run over the whole canvas rather than
    /// per row: Zork Zero answers 320/320 and Arthur 320/320 — both flanks meeting in
    /// the middle of the screen, with no story left between them.
    #[test]
    fn a_flanks_columns_come_from_the_narrowest_row_of_its_art() {
        const W: u32 = 640;
        const H: u32 = 400;
        let opaque = image::Rgba([1u8, 2, 3, 255]);
        let paint = |c: &mut image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32| {
            for y in y0..y1 {
                for x in x0..x1 {
                    c.put_pixel(x, y, opaque);
                }
            }
        };
        // Scale 1.0, no letterbox offset, an 8x18 cell: the native→cell arithmetic is
        // a plain divide, so the expected numbers are readable.
        let scale = crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 };
        let pane = Rect::new(0, 0, 80, 30);
        let columns =
            |c: &image::RgbaImage| flank_art_columns(c, &scale, (8, 18), pane, zvm::screen::V6Cell::DEFAULT);

        // No art at all: the flank has nothing to say and the whole pane is left to
        // the caller's own answer.
        let bare = image::RgbaImage::new(W, H);
        assert_eq!(columns(&bare), (pane.x, pane.right()), "an empty canvas claims no columns");

        // Shogun: 46-wide ornament at each edge, all 400 rows.
        let mut shogun = image::RgbaImage::new(W, H);
        paint(&mut shogun, 0, 46, 0, H);
        paint(&mut shogun, 594, W, 0, H);
        assert_eq!(columns(&shogun), (6, 74), "46 native → ceil(46/8)=6; 594 → floor(594/8)=74");

        // Zork Zero: pillars under a full-width banner.
        let mut zork = image::RgbaImage::new(W, H);
        paint(&mut zork, 0, W, 0, 72); // the banner, edge to edge
        paint(&mut zork, 0, 72, 72, H); // left pillar
        paint(&mut zork, 566, W, 72, H); // right pillar
        assert_eq!(
            columns(&zork),
            (9, 70),
            "the banner spans the screen and must not widen the pillars: 72 → ceil(72/8)=9, \
             566 → floor(566/8)=70"
        );

        // Arthur: poles ABUTTING a header, plus the transparent gutter at the very edge
        // his artwork really has.
        let mut arthur = image::RgbaImage::new(W, H);
        paint(&mut arthur, 4, 28, 0, 384); // left pole
        paint(&mut arthur, 612, 636, 0, 384); // right pole
        paint(&mut arthur, 28, 612, 0, 192); // header, flush against both poles
        assert_eq!(
            columns(&arthur),
            (4, 76),
            "the header abuts the poles, so its rows offer one run clear across the screen; \
             only the pole-only rows are narrow: 28 → ceil(28/8)=4, 612 → floor(612/8)=76"
        );

        // SQ-0899, the two shapes that are not flanks at all. Arthur's ProDOS press is
        // the first and mysterious01 the second, and each was read as a pair of side
        // columns before this: the first tiled a sliver of the picture down 67 cells of
        // both flanks, and the second is only harmless today because both its answers
        // land on the pane's middle and the caller throws the pair away.
        // Hollow, as the real plate is: its narrowest rows offer a THREE-pixel run, so
        // the reduction has a plausibly column-shaped answer to hand back and only the
        // distance from the edge tells it apart from a pole.
        let mut centred = image::RgbaImage::new(W, H);
        paint(&mut centred, 250, 253, 104, 296); // the illustration's left stroke
        paint(&mut centred, 387, 390, 104, 296); // …and its right
        paint(&mut centred, 250, 390, 104, 108); // its top edge, solid across
        assert_eq!(
            columns(&centred),
            (pane.x, pane.right()),
            "a picture in the MIDDLE of the screen is not a side column, however narrow the \
             run in from the edge makes it look"
        );
        let mut full = image::RgbaImage::new(W, H);
        paint(&mut full, 0, W, 0, H); // one picture over the whole screen
        assert_eq!(
            columns(&full),
            (pane.x, pane.right()),
            "a run that reaches the canvas MIDDLE was stopped by the scan's own bound, not by \
             the art, and says nothing about a column's width"
        );
    }

    /// SQ-0818's whole safety argument, as a property: the tiles of a band PARTITION
    /// it. Every column of the strip is covered by exactly one tile — a gap would
    /// leave a column of the ring unwritten, an overlap would put two images on one
    /// cell — every tile is at least one cell wide and no wider than the unit, and
    /// every tile keeps the band's own rows. Story-free, so it holds in CI where the
    /// gitignored fixtures are absent and the real-game smoke skips.
    #[test]
    fn band_tiles_partition_the_band_exactly() {
        for width in 1u16..=200 {
            for unit in [0u16, 1, 3, 8, 16] {
                let band = Rect::new(7, 3, width, 5);
                let tiles = band_tiles(band, unit);
                assert!(!tiles.is_empty(), "w={width} unit={unit}: a band is always drawn");
                let mut x = band.x;
                for t in &tiles {
                    assert_eq!(t.x, x, "w={width} unit={unit}: no gap, no overlap: {tiles:?}");
                    assert!(t.width >= 1, "w={width} unit={unit}: every tile is at least a cell: {tiles:?}");
                    assert_eq!((t.y, t.height), (band.y, band.height), "w={width} unit={unit}: rows preserved");
                    assert!(unit == 0 || t.width <= unit, "w={width} unit={unit}: no tile exceeds the unit: {tiles:?}");
                    x += t.width;
                }
                assert_eq!(x, band.right(), "w={width} unit={unit}: the tiles reach the right edge: {tiles:?}");
                if unit == 0 {
                    assert_eq!(tiles, vec![band], "unit 0 disables tiling — the backends that must not tile");
                } else {
                    assert_eq!(tiles.len(), (width.div_ceil(unit)) as usize, "w={width} unit={unit}: tile count");
                }
            }
        }
    }

    /// Real-Zork0 raster acceptance (SQ-0510): compose the raster canvas exactly
    /// as the raster branch does, then prove the finished image is fully opaque,
    /// that the story page and the ink are distinct, and that not one artwork
    /// pixel was painted over. Skips cleanly when the gitignored story is absent.
    #[test]
    fn zork0_raster_canvas_is_opaque_and_preserves_art() {
        use crate::render::v6_layout as v6;
        let story_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork0-r393-s890714.z6");
        let Ok(bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return;
        };
        let mut picts = crate::graphics::PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
        let dims = picts.all_pict_dims();
        let mut session =
            crate::session::GameSession::new_with_trace(bytes, false, false, None, false, dims, picts.std_window(), None, None)
                .expect("Zork0 (v6) loads and boots");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let model = crate::engine::Engine::screen(&session);
        let items = match &model.root {
            WinNode::Layered(v) => v.clone(),
            other => panic!("expected Layered, got {other:?}"),
        };

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript("West of House");
        state.push_transcript("You are standing in an open field west of a white house.");

        // The user's dark terminal: OSC 10/11 both answered → the pair is the
        // terminal's own light ink on its dark page.
        let osc = crate::term_colors::TermDefaultColors {
            fg: Some(image::Rgba([216, 216, 216, 255])),
            bg: Some(image::Rgba([26, 26, 26, 255])),
        };
        let (ink, page_default) =
            v6_default_pair(state.colors.theme.get("transcript").style, osc.fg, osc.bg);
        assert_ne!(ink, page_default, "ink and page must never resolve to the same colour");

        // ── Compose exactly as the raster branch does ────────────────────────
        let native = v6::native_extent(&items, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = v6::classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
        let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, ink, page_default, &state.colors, v6::TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let chrome_only = canvas.clone(); // pre-fill artwork reference
        let page = v6::story_bg_rgba(layout.story, &state.colors).unwrap_or(page_default);
        let (sx, sy, sw, sh) = v6::story_clear_native(layout.story, &canvas).expect("Zork0 has a story window");
        assert!(sw > 0 && sh > 0, "Zork0's clear story interior is non-empty: {sw}x{sh}");
        v6::fill_cell(&mut canvas, sx, sy, sw, sh, page);
        let cols = (sw / 8).max(1) as u16;
        let rows = (sh / 16).max(1) as u16;
        let (main, _) = build_main_text(&state, cols, rows);
        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, ink, &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
        let pre_flatten = canvas.clone();
        v6::flatten_onto_page(&mut canvas, page);

        // (1) The shipped image is fully opaque: no pixel is left for a compositor
        // (kitty's terminal backdrop, halfblocks' white `Color::Reset`) to resolve.
        assert!(
            canvas.pixels().all(|p| p[3] == 255),
            "every pixel of the raster composite is opaque"
        );

        // (2) Nothing already drawn was painted over — every pixel any layer had
        // touched is byte-identical after the flatten (frame art, banner text,
        // status bands, glyphs, and any inline drop-cap alike).
        for (x, y, p) in pre_flatten.enumerate_pixels() {
            if p[3] > 0 {
                assert_eq!(canvas.get_pixel(x, y), p, "flatten must not repaint drawn pixel ({x},{y})");
            }
        }
        // ...and the frame artwork specifically still matches the pre-fill chrome.
        let mut art_pixels = 0u32;
        for (x, y, p) in chrome_only.enumerate_pixels() {
            if p[3] > 0 {
                art_pixels += 1;
                assert_eq!(canvas.get_pixel(x, y), p, "frame art pixel ({x},{y}) survives fill+flatten");
            }
        }
        assert!(art_pixels > 10_000, "Zork0's frame art is substantial: {art_pixels} px");

        // (3) The story interior reads as the resolved page, and the text on it is
        // visible (glyph ink differs from the page).
        assert_eq!(*canvas.get_pixel(sx + sw / 2, sy + sh / 2), page, "story interior is the opaque page");
        let ink_px = canvas
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                (sx..sx + sw).contains(x) && (sy..sy + sh).contains(y) && **p == ink
            })
            .count();
        assert!(ink_px > 100, "seeded story text is drawn in ink on the page: {ink_px} ink pixels");
    }

    #[test]
    fn v6_default_pair_resolves_ink_and_page_from_one_source() {
        use ratatui::style::{Color, Style};
        let osc_fg = Some(image::Rgba([10, 20, 30, 255]));
        let osc_bg = Some(image::Rgba([40, 50, 60, 255]));
        let osc_pair = (image::Rgba([10, 20, 30, 255]), image::Rgba([40, 50, 60, 255]));
        let fallback = (RASTER_FALLBACK_INK, RASTER_FALLBACK_PAGE);

        // (a) Theme supplies BOTH channels → the theme pair, OSC ignored.
        let both = Style::default().fg(Color::Rgb(1, 2, 3)).bg(Color::Rgb(4, 5, 6));
        assert_eq!(
            v6_default_pair(both, osc_fg, osc_bg),
            (image::Rgba([1, 2, 3, 255]), image::Rgba([4, 5, 6, 255]))
        );

        // (b) THE REGRESSION: theme supplies fg ONLY (a cream ink with no page) and
        // OSC answered both → the OSC pair, NOT the theme ink mixed with an OSC page.
        let fg_only = Style::default().fg(Color::Rgb(1, 2, 3));
        assert_eq!(v6_default_pair(fg_only, osc_fg, osc_bg), osc_pair);
        // Symmetric partiality: theme supplies bg ONLY → still skipped whole.
        let bg_only = Style::default().bg(Color::Rgb(4, 5, 6));
        assert_eq!(v6_default_pair(bg_only, osc_fg, osc_bg), osc_pair);

        // (c) No theme RGB at all + OSC answered both → the OSC pair.
        let unset = Style::default();
        assert_eq!(v6_default_pair(unset, osc_fg, osc_bg), osc_pair);

        // (d) Only ONE OSC channel answered → the fallback pair (no mixing).
        assert_eq!(v6_default_pair(unset, osc_fg, None), fallback);
        assert_eq!(v6_default_pair(unset, None, osc_bg), fallback);

        // (e) Nothing → the fallback pair.
        assert_eq!(v6_default_pair(unset, None, None), fallback);

        // A named (non-Rgb) theme colour is not "supplied" — the pixel canvas needs
        // real bytes, so terminal_default White/Black falls through to the OSC pair.
        let named = Style::default().fg(Color::White).bg(Color::Black);
        assert_eq!(v6_default_pair(named, osc_fg, osc_bg), osc_pair);
    }

    #[test]
    fn content_bounds_never_clamps_a_layered_v6_root() {
        // The v6 raster/hybrid paths scale pixel content to the pane; clamping
        // to the cell content_size pinned the game to a native-size stamp in
        // the corner of a large terminal (the live "tiny render" bug).
        let model = hybrid_v6_model(); // Layered root, content_size (40, 25)
        let area = Rect::new(0, 0, 210, 55);
        assert_eq!(content_bounds(&model, area), area, "Layered root gets the full pane");
    }

    /// The live input line and its caret must NOT depend on which pane holds the
    /// keyboard. This was focus-gated on the Glulx/v6 raster path, so opening a room
    /// panel (or reaching the inspector via select-room) made the caret and your
    /// half-typed command disappear with no sign they were still buffered — while the
    /// Z-machine transcript path, which has no such gate, kept showing them.
    #[test]
    fn the_live_input_line_shows_regardless_of_which_pane_has_focus() {
        let cols = 40u16;
        let rows = 10u16;
        for focus in [crate::state::Focus::Game, crate::state::Focus::Map] {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.push_transcript("You are standing in an open field.");
            for ch in "open mailbox".chars() {
                state.input.value.push(ch);
            }
            state.input.cursor = state.input.value.chars().count();
            state.focus = focus;

            let (main, _) = build_main_text(&state, cols, rows);
            assert!(
                main.awaiting,
                "the input line must render with focus {focus:?} — it is not a focus indicator"
            );
            assert_eq!(main.input, "open mailbox", "the buffered command must be carried through");
            assert_eq!(main.cursor_col, "open mailbox".chars().count() as u16, "caret sits after the text");
        }
    }

    #[test]
    fn build_main_text_floats_inline_image_and_narrows_beside_it() {
        // A transcript-anchored inline image (32×64 → 4 rows at the DEFAULT cell's
        // 16-pixel height — this case builds its own fixture there, margin
        // 40px → 5 cols) becomes a float: it occupies no text row, the 4 rows
        // beside it wrap narrower, and rows past it wrap at full width.
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(32, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(40),
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image line became a float, not a text row");
        let f = &main.floats[0];
        assert_eq!((f.row, f.rows, f.reserve_cols, f.text_col, f.img_col), (1, 4, 5, 5, 0), "anchored after 'before', 64px/16 = 4 rows, 40px/8 = 5 cols, left float");
        assert_eq!(main.lines[0], "before");
        // Rows 1..5 (beside the float) wrap at 40-5=35 cols; later rows full width.
        for (i, row) in main.lines.iter().enumerate().skip(1) {
            let w = row.chars().count();
            if (1..5).contains(&i) {
                assert!(w <= 35, "row {i} beside the float is narrow, got {w}");
            } else {
                assert!(w <= 40, "row {i} is full width, got {w}");
            }
        }
        assert!(main.lines[5..].iter().any(|r| r.chars().count() > 35), "rows past the float use full width");
    }

    #[test]
    fn build_main_text_bands_inline_up_content_art_full_width() {
        // Shogun's opening ship illustration is a window-0 picture classified
        // InlineUp by `win0_pic_align` (content-art sized, SQ-0471) — unlike a
        // MarginLeft drop-cap it must NOT float with text wrapped beside it: it
        // reserves full-width blank rows, text stops above and resumes below,
        // never over it (SQ-0470 follow-up).
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(160, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
            margin_px: None,
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image still carries its pixels for the canvas blit");
        let f = &main.floats[0];
        // 64px / 16 (the default cell) = 4 rows, anchored right after "before" (row 1).
        assert_eq!((f.row, f.rows), (1, 4), "band anchored after 'before', 64px/16 = 4 rows");
        assert_eq!(main.lines[0], "before");
        // Every row the band spans is blank — no text row overlaps its rows.
        for (i, row) in main.lines.iter().enumerate() {
            if (f.row as usize..f.row as usize + f.rows as usize).contains(&i) {
                assert!(row.is_empty(), "row {i} is inside the band and must carry no text, got {row:?}");
            }
        }
        // Text resumes below the band, at full (unindented) width — long enough
        // to prove it isn't narrowed the way a MarginLeft float would narrow it.
        assert!(!main.lines[5].is_empty(), "text resumes right after the band");
        assert!(main.lines[5].chars().count() > 35, "row 5 is full width, got {:?}", main.lines[5]);
    }

    #[test]
    fn build_main_text_right_float_narrows_left_and_places_picture_right() {
        // SQ-0489: Shogun's opening — a MarginRight window-0 picture floats at the
        // RIGHT edge; the prose rows beside it stay flush LEFT but narrow, and rows
        // past the picture reclaim full width. (160px → 20 cols; reserve 21; in a
        // 40-col box the text column is 19 cols.)
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(160, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::MarginRight,
            scaled: None,
            margin_px: None,
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image became a right float");
        let f = &main.floats[0];
        // 64px/16 = 4 rows; 160px/8 = 20 img cols; reserve = 21; img_col = 40-20 = 20.
        assert_eq!((f.row, f.rows, f.reserve_cols, f.text_col, f.img_col), (1, 4, 21, 0, 20), "right float geometry");
        assert_eq!(main.lines[0], "before");
        // Rows 1..5 (beside the float) wrap at 40-21 = 19 cols; later rows widen.
        for (i, row) in main.lines.iter().enumerate().skip(1) {
            let w = row.chars().count();
            if (1..5).contains(&i) {
                assert!(w <= 19, "row {i} beside the right float is narrow, got {w}");
            }
        }
        assert!(main.lines[5..].iter().any(|r| r.chars().count() > 19), "rows past the float use full width");
    }

    #[test]
    fn build_main_text_maps_style_runs_onto_wrapped_rows() {
        // SQ-0540: the raster prose path carries per-char emphasis, so the
        // synthesized bold/italic faces land on the same characters the terminal
        // transcript emphasises. The mapping must survive word wrapping: a run's
        // char offsets index the UNWRAPPED line.
        let mut state = crate::state::AppState::default();
        // 3 words of 5 chars: "aaaaa bbbbb ccccc" wraps to 2 rows at 12 cols.
        state.push_transcript_kind("aaaaa bbbbb ccccc", crate::state::TranscriptKind::Story);
        state.transcript_runs.resize(state.transcript.len(), Vec::new());
        let last = state.transcript.len() - 1;
        state.transcript_runs[last] = vec![
            // "bbbbb" bold (chars 6..11), the trailing "cc" of "ccccc" italic.
            crate::state::StyleRun { start: 6, end: 11, bits: 2, fg: 0, bg: 0, link: 0, glk_style: 0 },
            crate::state::StyleRun { start: 15, end: 17, bits: 4, fg: 0, bg: 0, link: 0, glk_style: 0 },
        ];
        let (main, _) = build_main_text(&state, 12, 8);
        assert_eq!(main.lines, vec!["aaaaa bbbbb", "ccccc"], "wraps into two rows");
        assert_eq!(main.styles.len(), main.lines.len(), "styles stay parallel to lines");
        assert_eq!(main.styles[0], vec![0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2], "row 0: only 'bbbbb' is bold");
        assert_eq!(main.styles[1], vec![0, 0, 0, 4, 4], "row 1: the run is rebased past the dropped wrap space");

        // Reverse/fixed-pitch bits are dropped (no block to swap in the prose
        // raster, and the bitmap font is fixed-pitch already) — and a line with
        // no emphasis at all allocates no style row.
        state.transcript_runs[last] = vec![crate::state::StyleRun { start: 0, end: 17, bits: 1 | 8, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        let (main, _) = build_main_text(&state, 12, 8);
        assert!(main.styles.iter().all(|r| r.is_empty()), "reverse/fixed-pitch leave every row roman, got {:?}", main.styles);
    }

    #[test]
    fn build_main_text_honors_transcript_scroll_offset() {
        // 20 short story lines into a 6-row story box (budget = 5 body rows). The
        // visible slice must window by `effective_transcript_scroll` (rows from the
        // bottom), clamped to `max_scroll`, newest-at-bottom when the offset is 0.
        let mut state = crate::state::AppState::default();
        for k in 0..20 {
            state.push_transcript_kind(&format!("L{k}"), crate::state::TranscriptKind::Story);
        }
        // Offset 0: the newest 5 rows (L15..=L19).
        state.transcript_scroll = 0;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(m.total_rows, 20);
        assert_eq!(m.viewport_rows, 5, "6 story-box rows minus the input line");
        assert_eq!(m.max_scroll, 15, "20 total - 5 body");
        assert_eq!(main.lines, vec!["L15", "L16", "L17", "L18", "L19"]);
        assert_eq!(m.first_visible_row, 15);

        // Scrolled back 3: the window shifts up by 3 (L12..=L16).
        state.transcript_scroll = 3;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(main.lines, vec!["L12", "L13", "L14", "L15", "L16"]);
        assert_eq!(m.first_visible_row, 12);

        // Over-scroll past the top clamps to max_scroll: the oldest 5 rows.
        state.transcript_scroll = 999;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(main.lines, vec!["L0", "L1", "L2", "L3", "L4"]);
        assert_eq!(m.first_visible_row, 0);
    }

    #[test]
    fn build_main_text_short_transcript_shows_all_and_never_scrolls() {
        // Fewer wrapped rows than the budget: everything is visible, max_scroll is
        // 0, and any scroll offset is a no-op (the view stays pinned at the bottom).
        let mut state = crate::state::AppState::default();
        for k in 0..3 {
            state.push_transcript_kind(&format!("L{k}"), crate::state::TranscriptKind::Story);
        }
        state.transcript_scroll = 7; // clamped to 0
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(m.total_rows, 3);
        assert_eq!(m.max_scroll, 0, "content fits — nothing to scroll");
        assert_eq!(main.lines, vec!["L0", "L1", "L2"]);
        assert_eq!(m.first_visible_row, 0);
    }

    /// Build a `Theme` with the given selectors' bg overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to
    /// `theme.get("<selector>")` (SQ-0309) can still inject a custom colour
    /// instead of mutating the (no-longer-read) legacy `ColorScheme` field.
    fn theme_with_bg_overrides(overrides: &[(&str, ratatui::style::Color)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, bg) in overrides {
            decls.insert(sel.to_string(), crate::theme::registry::Delta { bg: Some(bg), ..crate::theme::registry::Delta::EMPTY });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    #[test]
    fn reserve_text_margin_insets_caps_and_noops_at_zero() {
        let mut state = crate::state::AppState::default();
        let fill = ratatui::style::Style::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        let area = Rect::new(0, 0, 20, 10);

        // Zero margin returns the rect untouched.
        state.config.text_margin_x = 0;
        state.config.text_margin_y = 0;
        assert_eq!(reserve_text_margin(area, &state, fill, &mut buf), area);

        // (2,1) reserves 2 columns each side and 1 row top+bottom.
        state.config.text_margin_x = 2;
        state.config.text_margin_y = 1;
        assert_eq!(reserve_text_margin(area, &state, fill, &mut buf), Rect::new(2, 1, 16, 8));

        // An over-large margin is capped so at least one cell of text survives.
        state.config.text_margin_x = 100;
        state.config.text_margin_y = 100;
        let got = reserve_text_margin(area, &state, fill, &mut buf);
        assert!(got.width >= 1 && got.height >= 1, "capped margin keeps >=1 cell: {got:?}");
    }

    #[test]
    fn simple_path_transcript_geometry_is_inset_by_text_margin() {
        // The rect render_transcript publishes as `transcript_geom` (what mouse
        // selection maps through) must shrink by exactly the configured margin, so
        // the inset stays consistent with clicks and the copy path (SQ-0345).
        let published = |mx: u16, my: u16| {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.config.text_margin_x = mx;
            state.config.text_margin_y = my;
            for k in 0..5 { state.push_transcript(&format!("line {k}")); }
            let model = ScreenModel {
                root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
                status: StatusModel::HostManaged,
                bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
                fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
                content_size: (0, 0),
            };
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            state.transcript_geom.get().expect("transcript geom published").area
        };
        let base = published(0, 0);
        let inset = published(3, 2);
        assert_eq!(inset.x, base.x + 3, "left margin reserved");
        assert_eq!(inset.y, base.y + 2, "top margin reserved");
        assert_eq!(inset.width, base.width - 6, "both horizontal margins reserved");
        assert_eq!(inset.height, base.height - 4, "top+bottom margins reserved");
    }

    /// SQ-0532/A-F1. ZMSD §8.4: the interpreter "may change the exact dimensions
    /// whenever it likes but must write the current height (in lines) and width
    /// (in characters) into bytes $20 and $21 in the header." What we report is
    /// therefore MEASURED from the story pane, not a fixed 80x24 guess.
    #[test]
    fn story_screen_dims_measure_the_story_pane() {
        let state = frameless_state(); // upper-window frame themed off
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((30, 99)),
            "a bare pane reports its own cell size, less the scrollbar gutter column"
        );
        assert_eq!(
            story_screen_dims(Rect::new(4, 2, 62, 17), &state),
            Some((17, 61)),
            "the pane's position is irrelevant; only its extent is reported"
        );
        // A hidden or not-yet-measured pane has nothing to report.
        assert_eq!(story_screen_dims(Rect::new(0, 0, 0, 0), &state), None);
        assert_eq!(story_screen_dims(Rect::new(0, 0, 80, 0), &state), None);
    }

    /// The declared width has to follow the map pane DISAPPEARING (SQ-1084).
    ///
    /// This is a guard on a fact that is invisible at the call site rather than on
    /// arithmetic. `story_screen_dims` was never wrong; what was wrong was the pane
    /// handed to it at boot. `startup::pre_boot_host_screen` synthesises an
    /// `AppState` to ask "how wide is the story pane before there is a state", and
    /// it set four fields on it and not `layout` — so `compute_pane_layout` split
    /// the frame for a visible map every time, and any story whose map the player
    /// had hidden was told HALF the terminal it actually had. Nothing looked wrong:
    /// prose is re-wrapped by us and has no leading run to misplace, so the damage
    /// showed only where a game centres or indents with spaces of its own — a title
    /// screen, a menu, an epigraph — which then sat centred in the left half of a
    /// full-width pane. Measured on Anchorhead at 100 columns: column 16 against
    /// `zvm-cli`'s 40.
    ///
    /// So this asserts the RELATIONSHIP, not two numbers: hiding the map must widen
    /// the declared screen by most of the frame. A future layout change may move
    /// either figure; it must not make them equal.
    #[test]
    fn the_declared_width_follows_the_map_pane_being_hidden() {
        let frame = Rect::new(0, 0, 100, 32);
        let mut state = frameless_state();

        state.layout = crate::state::Layout::Split;
        let split = crate::layout::compute_pane_layout(frame, &state, 0);
        let (_, with_map) = story_screen_dims(split.story, &state).expect("a split pane");

        state.layout = crate::state::Layout::TranscriptFull;
        let full = crate::layout::compute_pane_layout(frame, &state, 0);
        let (_, without_map) = story_screen_dims(full.story, &state).expect("a full pane");

        assert!(
            without_map > with_map + 30,
            "hiding the map must widen the declared screen: got {without_map} without the map \
             against {with_map} with it, on a 100-column frame. Equal or nearly-equal numbers mean \
             the layout is being measured for a visible map whatever the state says, which is \
             SQ-1084 — a story then centres its title for half the screen it has"
        );
        assert!(
            without_map >= 90,
            "with the map hidden the story pane is the whole frame less chrome, so the declared \
             width should be near 100 on a 100-column frame; got {without_map}"
        );
    }

    #[test]
    fn story_screen_dims_subtract_the_margin_and_the_upper_window_frame() {
        // The declared screen is the region the game's own screen actually gets:
        // the text margin is where the transcript wraps, and the upper window's
        // frame is drawn AROUND the grid, so both come off the reported size.
        let mut state = frameless_state();
        state.config.text_margin_x = 3;
        state.config.text_margin_y = 2;
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((30, 93)),
            "horizontal margin comes off the width; the grid is never inset vertically"
        );
        state.config.text_margin_x = 0;
        state.colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single);
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((28, 97)),
            "a framed upper window loses one row/column per drawn side"
        );
    }

    #[test]
    fn story_screen_dims_honour_a_pinned_config_override() {
        // `virtual_screen_cols`/`rows` stay available for pinning a fixed virtual
        // screen; an unset key follows the pane (see the config docs).
        let mut state = frameless_state();
        state.config.virtual_screen_cols = Some(80);
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 132, 40), &state),
            Some((40, 80)),
            "a pinned width wins; the unset height still follows the pane"
        );
        state.config.virtual_screen_rows = Some(24);
        assert_eq!(story_screen_dims(Rect::new(0, 0, 132, 40), &state), Some((24, 80)));
    }

    /// SQ-0679: the width DECLARED to a v4+ story never drops below the width
    /// it booted at, because a v4/v5 status routine reads $21 once and bakes
    /// its field columns in — narrow the screen under it and those columns
    /// fall outside the window, where §8.7.2.3 makes the `set_cursor` illegal and
    /// the digits land on the room name instead. Widening still follows the pane
    /// (SQ-0533), and the HEIGHT always does: `split_window` re-declares it on
    /// every layout.
    ///
    /// SQ-0680: the floor is `boot_cols`, THIS session's actual boot width —
    /// `zvm::screen::DEFAULT_SCREEN_COLS` (80) unseeded, matching the original
    /// SQ-0679 assumption, or whatever narrower/wider pane the caller pre-boot
    /// seeded (`GameSession::boot_screen_cols`).
    #[test]
    fn declared_width_never_drops_below_the_boot_width() {
        let state = frameless_state();
        let narrow = Rect::new(0, 0, 60, 20);
        let wide = Rect::new(0, 0, 132, 40);
        let boot_80 = zvm::screen::DEFAULT_SCREEN_COLS as u16;
        // The raw pane measurement is unchanged — it still measures the pane.
        assert_eq!(story_screen_dims(narrow, &state), Some((20, 59)));

        // v5: floored at the boot width going down, free to follow the pane up.
        assert_eq!(
            declared_story_screen_dims(narrow, &state, 5, boot_80),
            Some((20, 80)),
            "a 59-column pane still declares the 80 columns the story booted with"
        );
        assert_eq!(
            declared_story_screen_dims(wide, &state, 5, boot_80),
            Some((40, 131)),
            "a wider pane is declared in full — every old coordinate is still inside it"
        );
        // The height follows the pane in both directions.
        assert_eq!(declared_story_screen_dims(narrow, &state, 5, boot_80).unwrap().0, 20);

        // v3 has no such header fields, and v6's screen is its native pixel
        // frame — neither is floored.
        assert_eq!(declared_story_screen_dims(narrow, &state, 3, boot_80), Some((20, 59)));
        assert_eq!(declared_story_screen_dims(narrow, &state, 6, boot_80), Some((20, 59)));

        // An explicitly pinned width is the user's, not ours to floor.
        let mut pinned = frameless_state();
        pinned.config.virtual_screen_cols = Some(40);
        assert_eq!(declared_story_screen_dims(narrow, &pinned, 5, boot_80), Some((20, 40)));

        // SQ-0680: a session pre-boot-seeded to a NARROWER pane floors at ITS
        // own boot width, not the fixed 80 default — a 60-column pane that
        // booted at 60 must not be forced back up to 80 on the next poll,
        // which would silently undo the whole point of seeding it.
        assert_eq!(
            declared_story_screen_dims(narrow, &state, 5, 60),
            Some((20, 60)),
            "a 59-column pane under a 60-column boot floors at the boot width, not 80"
        );
        // …and a pane exactly at (or wider than) that boot width is reported
        // as-measured, same as always.
        assert_eq!(declared_story_screen_dims(wide, &state, 5, 60), Some((40, 131)));
    }

    /// SQ-0532/A-F1(c): the width the story is TOLD about, the width the upper
    /// window is RENDERED at, and the width the transcript WRAPS at are one
    /// number. Before this, the grid was sized from a fixed 80-column header and
    /// centred in the pane while the prose wrapped at the pane's real width, so a
    /// game's full-width form sat offset from the text beside it.
    #[test]
    fn declared_width_equals_rendered_grid_width_equals_transcript_wrap() {
        let state = frameless_state(); // no upper-window frame, no text margin
        let area = Rect::new(0, 0, 60, 12);
        let (_, cols) = story_screen_dims(area, &state).expect("pane measured");
        assert_eq!(cols, area.width - 1, "with no frame and no margin, the pane less its scrollbar gutter");

        // A grid sized the way `split_window` sizes it — from header byte $21.
        let mut grid = crate::engine::GridWindow { active_rows: 1, ..Default::default() };
        grid.resize(1, cols);
        grid.put(1, 1, '<', 0);
        grid.put(1, cols, '>', 0);
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let used = draw_upper_window(&grid, false, &state.colors, area, &mut buf, true, &mut links);
        assert_eq!(used, 1, "one grid row, no frame rows");
        // Rendered edge-to-edge across the pane: no centring offset left to drift.
        assert_eq!(buf.cell((area.x, area.y)).unwrap().symbol(), "<");
        assert_eq!(buf.cell((area.x + cols - 1, area.y)).unwrap().symbol(), ">");

        // The transcript below wraps at that same width (its rightmost column is
        // the scrollbar gutter, which is chrome, not story columns).
        let mut state2 = frameless_state();
        state2.push_transcript("x");
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height - used);
        let _ = render_transcript(&model.status, None, &state2, tarea, &mut buf, None);
        let geom = state2.transcript_geom.get().expect("geometry published").area;
        assert_eq!(geom.width, cols, "transcript wraps at the declared width");
        assert_eq!(geom.x, area.x, "and starts at the same column the grid does");
    }

    #[test]
    fn scrollbar_sits_at_border_not_inside_text_margin() {
        // With a horizontal text margin, only the text is inset — the scrollbar
        // must stay flush against the pane border (rightmost column), never inside
        // the margin band (SQ-0345).
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let mx = 3;
        state.config.text_margin_x = mx;
        // Far more lines than the viewport → scrollbar must appear.
        for k in 0..80 { state.push_transcript(&format!("line {k}")); }
        state.scroll_transcript_to(1); // SQ-0782: the story bar shows because you scrolled
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
            content_size: (0, 0),
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // SQ-0782: the bar is a background fill, so look for its colour, not a glyph.
        let thumb = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme).thumb;
        let painted = |x: u16| (0..area.height).any(|y| buf.cell((x, y)).unwrap().bg == thumb);
        assert!(painted(area.width - 1), "the bar should paint the border column, not one inset by the margin");
        // And the inset column (where the scrollbar used to sit) must be clear of it.
        assert!(!painted(area.width - 1 - mx), "no scrollbar inside the margin band");
    }

    /// SQ-0782: the bar is app chrome, so it keeps its theme colours whatever
    /// the story painted the page — pinned in BOTH `honor_game_colours` modes
    /// (true is the shipped default), because a game page colour flooding the
    /// pane must not repaint the gutter with it.
    #[test]
    fn scrollbar_keeps_its_theme_colours_in_both_honor_game_colours_modes() {
        let mut colours = Vec::new();
        for honor in [true, false] {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.config.honor_game_colours = honor;
            for k in 0..80 { state.push_transcript(&format!("line {k}")); }
            state.scroll_transcript_to(1);
            // The story has set a page scheme (white on blue).
            let model = ScreenModel {
                root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
                status: StatusModel::HostManaged,
                bg: crate::state::pack_zcolour(zvm::screen::ZColour::Standard(6)),
                fg: crate::state::pack_zcolour(zvm::screen::ZColour::Standard(9)),
                content_size: (0, 0),
            };
            let area = Rect::new(0, 0, 40, 12);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            let look = crate::render::scroll::ScrollbarLook::from_theme(&state.colors.theme);
            let bgs: Vec<_> = (0..area.height)
                .map(|y| buf.cell((area.width - 1, y)).unwrap().bg)
                .filter(|c| *c == look.thumb || *c == look.track)
                .collect();
            assert!(!bgs.is_empty(), "honor={honor}: the bar draws in its theme colours");
            colours.push(bgs);
        }
        assert_eq!(colours[0], colours[1], "the bar looks the same in both modes");
    }

    #[test]
    fn garglk_margin_overrides_config_default() {
        // A discovered garglk.ini's tmargin wins over the global config margin
        // (SQ-0344, highest precedence).
        let mut state = crate::state::AppState::default();
        state.config.text_margin_x = 0;
        state.config.text_margin_y = 0;
        state.garglk_overlay = Some(crate::garglk_ini::GarglkOverlay {
            margin_x: Some(3),
            margin_y: Some(1),
            ..Default::default()
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        let got = reserve_text_margin(Rect::new(0, 0, 20, 10), &state, ratatui::style::Style::default(), &mut buf);
        assert_eq!(got, Rect::new(3, 1, 14, 8), "garglk tmargin applied over the zero config default");
    }

    fn grid_with(text: &str) -> GridWindow {
        let mut g = GridWindow::default();
        g.resize(1, text.chars().count() as u16);
        for (i, ch) in text.chars().enumerate() {
            g.put(1, i as u16 + 1, ch, 0);
        }
        g.active_rows = 1;
        g
    }

    fn model_with_page(bg: zvm::screen::ZColour, fg: zvm::screen::ZColour) -> ScreenModel {
        ScreenModel {
            root: WinNode::Blank,
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(bg),
            fg: crate::state::pack_zcolour(fg),
            content_size: (0, 0),
        }
    }

    // ── the machine's page, before the story names one (SQ-0935) ────────────

    /// Flood the pane and report the background every cell came out with, or
    /// `None` where they disagree — the pane page is one colour or it is nothing.
    fn pane_page(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Color> {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let _ = render_story_pane(model, false, None, state, area, &mut buf);
        let first = buf.cell((0, 0))?.style().bg?;
        (0..area.height)
            .all(|y| (0..area.width).all(|x| buf.cell((x, y)).and_then(|c| c.style().bg) == Some(first)))
            .then_some(first)
    }

    /// A licensed launch presents the machine's page from the first frame, without
    /// waiting for the story to name it — and it is the MACHINE's own RGB.
    ///
    /// Shogun r322 is why. Its window 0 carries `(0, 0)` — inherit on both channels
    /// — from boot, through the whole opening and through InvisiClues, and names
    /// `Standard(6)` under `Standard(9)` only when it LEAVES the hint menu and
    /// restores the screen from the header. A game written for a DOS machine has no
    /// reason to name its page: the screen it prints on is already blue.
    ///
    /// **The shade is the machine's, not the theme's** (SQ-0935/SQ-0939). The IBM
    /// PC row resolves colour 6 through EGA — palette entry 1, `#0000AD` once the
    /// Z-machine's 15-bit space has been through it — which is what that number IS
    /// on that machine. Resolving it through the app's ColorScheme instead gave
    /// `#006BB5`, so the same machine showed one blue to a v3 story and another to a
    /// v6 one.
    ///
    /// **Not version-gated**, which is the change: the machine's screen applies to
    /// every version an Infocom interpreter shipped for. `app::period` holds the
    /// rule and the argument.
    #[test]
    fn a_licensed_machine_page_is_on_screen_before_the_story_names_one() {
        use zvm::screen::ZColour;
        for zversion in [3u8, 5, 6] {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.config.honor_game_colours = true;
            state.config.interpreter_profile = crate::interpreter::InterpreterProfile::IbmPc;
            state.config.interpreter_source = crate::interpreter::ProfileSource::Medium;
            state.story_zversion = Some(zversion);
            // Resolved the way `reload::reload_style` resolves it, so this exercises
            // the shipped gate rather than a hand-set field.
            state.period_look = crate::period::resolve(
                state.config.interpreter_profile,
                state.config.period_look,
                state.config.honor_game_colours,
                state.config.machine_colours_licensed(),
                state.story_zversion,
            );
            let look = state.period_look.unwrap_or_else(|| panic!("v{zversion}: a licensed DOS launch is dressed"));
            let machine_blue = ratatui::style::Color::Rgb(look.page.0, look.page.1, look.page.2);
            // The page IS the palette's resolution of the pair the row states —
            // one table lookup, not a second measurement (SQ-0939). Asserting that
            // identity rather than a literal is the point: it is what stops the
            // screen a story is painted on drifting from the colour the same story
            // gets out of `@set_colour(6)`.
            let (bg, fg) = state.config.machine_default_colours().expect("licensed");
            let via_palette = |n: u8| {
                let (r, g, b) = zvm::screen::rgb15_to_888(
                    zvm::screen::ega_true_colour(n, zversion == 6).expect("EGA carries 2..=9"),
                );
                ratatui::style::Color::Rgb(r, g, b)
            };
            assert_eq!(machine_blue, via_palette(bg), "the page is colour {bg} through EGA");
            let ink = ratatui::style::Color::Rgb(look.ink.0, look.ink.1, look.ink.2);
            assert_eq!(ink, via_palette(fg), "and the ink is colour {fg}");
            // …and THAT is why this sweeps versions: colour 9 is EGA 7 under XZIP
            // and EGA 15 under YZIP, so a v6 launch is white where v3 and v5 are grey.
            let expect_white = zversion == 6;
            assert_eq!(
                ink == ratatui::style::Color::Rgb(0xFF, 0xFF, 0xFF),
                expect_white,
                "v{zversion}: Infocom's two IBM interpreters disagree about white",
            );

            let model = model_with_page(ZColour::Default, ZColour::Default);
            assert_eq!(pane_page(&model, &state), Some(machine_blue), "v{zversion}: the machine's own page");
        }
    }

    /// …and it is a BASE COAT, not an override: the moment the story names a page,
    /// the story's wins. A v5 game that replaces the colours during its startup is
    /// still the authority on its own screen.
    #[test]
    fn a_story_that_names_a_page_overrides_the_machine_one() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        state.config.interpreter_profile = crate::interpreter::InterpreterProfile::IbmPc;
        state.config.interpreter_source = crate::interpreter::ProfileSource::Medium;
        state.story_zversion = Some(6);
        state.period_look = crate::period::resolve(
            state.config.interpreter_profile, state.config.period_look,
            state.config.honor_game_colours, state.config.machine_colours_licensed(),
            state.story_zversion,
        );
        let machine = state.period_look.expect("licensed").page;
        let model = model_with_page(ZColour::Standard(2), ZColour::Standard(9)); // black page
        let page = pane_page(&model, &state).expect("the game's page floods");
        assert_eq!(page, crate::render::resolve_zcolour(ZColour::Standard(2), &state.colors));
        assert_ne!(page, ratatui::style::Color::Rgb(machine.0, machine.1, machine.2), "not the machine's");
    }

    /// An UNLICENSED launch keeps the player's theme — the SQ-0928 rule this
    /// inherits. A bare story file off no disk never gets painted as a machine.
    #[test]
    fn an_unlicensed_launch_gets_no_machine_page() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        state.config.interpreter_profile = crate::interpreter::InterpreterProfile::IbmPc;
        state.config.interpreter_source = crate::interpreter::ProfileSource::Fallback;
        assert!(state.config.machine_default_colours().is_none(), "no medium named the machine");
        let model = model_with_page(ZColour::Default, ZColour::Default);
        let page = pane_page(&model, &state);
        let blue = crate::render::resolve_zcolour(ZColour::Standard(6), &state.colors);
        assert_ne!(page, Some(blue), "an unlicensed launch is not painted DOS blue");
    }

    /// `honor_game_colours = off` declines the machine's page with everything else.
    #[test]
    fn colours_declined_declines_the_machine_page_too() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = false;
        state.config.interpreter_profile = crate::interpreter::InterpreterProfile::IbmPc;
        state.config.interpreter_source = crate::interpreter::ProfileSource::Medium;
        let model = model_with_page(ZColour::Default, ZColour::Default);
        let blue = crate::render::resolve_zcolour(ZColour::Standard(6), &state.colors);
        assert_ne!(pane_page(&model, &state), Some(blue), "colours off means colours off");
    }

    #[test]
    fn grid_scheme_overrides_upper_window_with_game_page_colours() {
        // A game that set a black-on-white page (CounterfeitMonkey) → the grid base
        // becomes that page, so a reverse-video status line reverses to white-on-black
        // instead of reversing the app theme. (SQ-0262)
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Owned(_)), "override clone when the game set a scheme");
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)));
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn grid_scheme_also_paints_the_border_in_the_game_page_colours() {
        // SQ-0267: the status border is our own chrome (Glk sends no border style),
        // so it must adopt the game's page colours too — the whole status block
        // (content + frame) reads as one coloured unit on the recoloured page.
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));
        let gc = grid_scheme(&state, &model);
        assert_eq!(gc.theme.get("upper_window_border").style.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)),
            "border background matches the game page background");
        assert_eq!(gc.theme.get("upper_window_border").style.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)),
            "border line drawn in the game page foreground ink");
    }

    /// End-to-end guard for the same fix: render the simple (Z-machine) path with
    /// a game-set page scheme and check the actually-painted grid/border pixels,
    /// not just `grid_scheme`'s returned struct.
    #[test]
    fn simple_path_grid_and_border_paint_the_game_page_colours() {
        use ratatui::style::Color;
        let mut grid = grid_with("HI");
        grid.border = BorderPref::Border;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x00FF_FFFF)),
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0)),
            content_size: (0, 0),
        };
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // uw_w = 2 + 2 borders = 4, centered in 20 → x_off = 8; frame corner at
        // (8,0), content at (9,1) (mirrors `simple_path_still_frames_a_bordered_grid`).
        let border_cell = buf.cell((8, 0)).unwrap().style();
        assert_eq!(border_cell.fg, Some(Color::Rgb(0, 0, 0)), "border painted in the game page fg");
        assert_eq!(border_cell.bg, Some(Color::Rgb(255, 255, 255)), "border painted in the game page bg");
        let content_cell = buf.cell((9, 1)).unwrap().style();
        assert_eq!(content_cell.bg, Some(Color::Rgb(255, 255, 255)), "grid content painted in the game page bg");
    }

    // ── SQ-0510: the probe-seeded chrome vs. the game's own page ─────────────

    /// A `ColorScheme` whose theme was built the way `reload_style` builds it on
    /// a terminal that answered the OSC 10/11 probe with no scheme configured:
    /// chrome follows the terminal's real page instead of the hard-coded black.
    fn probe_seeded_colors() -> crate::colors::ColorScheme {
        let probe = crate::term_colors::TermDefaultColors {
            fg: Some(image::Rgba([0x58, 0x6e, 0x75, 255])),
            bg: Some(image::Rgba([0xfd, 0xf6, 0xe3, 255])),
        };
        let gs = crate::colors::seed_scheme_from_terminal(
            crate::colors::GhosttyScheme::default(),
            &probe,
        );
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.theme = crate::theme::resolve::resolve_theme(
            &gs,
            &crate::theme::toml_schema::ParsedStyle::default(),
        );
        cs
    }

    #[test]
    fn a_game_page_still_beats_the_probe_seeded_chrome() {
        // SQ-0262 must survive SQ-0510: seeding chrome from the terminal fixes the
        // case where NOBODY set a colour; a game that DOES set its page still owns
        // the grid. Run both `honor_game_colours` modes — with the gate on the game
        // wins, with it off the seeded terminal page stands (and neither is black).
        use ratatui::style::Color;
        use zvm::screen::ZColour;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));

        let mut state = AppState::default();
        state.colors = probe_seeded_colors();
        assert_eq!(
            state.colors.theme.get("upper_window").style.bg,
            Some(Color::Rgb(0xfd, 0xf6, 0xe3)),
            "precondition: the seeded chrome is the probed terminal page"
        );

        // honor = true (the shipped default): the game's white page wins outright.
        state.config.honor_game_colours = true;
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Owned(_)), "a game page still forces the override clone");
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(Color::Rgb(0, 0, 0)));
        assert_eq!(gc.theme.get("upper_window_border").style.bg, Some(Color::Rgb(255, 255, 255)));

        // honor = false: the game is ignored and the seeded terminal page stands —
        // still not the old hard-coded black.
        state.config.honor_game_colours = false;
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)));
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
    }

    #[test]
    fn a_game_that_sets_only_ink_keeps_the_probe_seeded_page() {
        // The half-set case `grid_scheme` has always handled: the game names an
        // ink but no page, so the page comes from the theme's chrome. That used to
        // mean black; with the probe answered it is the terminal's own page.
        use ratatui::style::Color;
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = probe_seeded_colors();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::Default, ZColour::True24(0x00FF_0000));

        let gc = grid_scheme(&state, &model);
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(Color::Rgb(255, 0, 0)), "the game's ink");
        assert_eq!(
            gc.theme.get("upper_window").style.bg,
            Some(Color::Rgb(0xfd, 0xf6, 0xe3)),
            "and the terminal's page beneath it"
        );
    }

    #[test]
    fn grid_scheme_borrows_theme_when_game_set_no_page() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let gc = grid_scheme(&state, &model_with_page(ZColour::Default, ZColour::Default));
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)), "theme unchanged when no game page colours");
    }

    #[test]
    fn grid_scheme_borrows_theme_when_colours_disabled() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = false;
        let gc = grid_scheme(&state, &model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0)));
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)), "game colours off → theme borrowed, override inert");
    }

    fn inline_buffer(line: &str) -> BufferWindow {
        BufferWindow {
            win: 0,
            lines: vec![line.to_string()],
            runs: vec![Vec::new()],
            para: vec![crate::state::ParaFmt::default()],
            images: vec![None],
            scroll: 0,
            primary: false,
            bg: None,
            fg: None,
            panel: false,
            px_runs: Vec::new(),
            reads_input: false,
        }
    }

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
    }

    /// SQ-0332: a deep, Kerkerkruip-shaped multi-window tree (nested bordered
    /// pairs, side panels + a main window + graphics-rule separators) renders with
    /// EVERY text pane painted in its own Normal-style background at its exact rect.
    /// Reconstructed from a live `/dump-windows` (165×60). Guards the render math
    /// (leaves must land on their dumped coordinates) and the per-window fills —
    /// the visible corruption came from a STALE tree (`bg = None`), not this path.
    #[test]
    fn deep_multiwindow_tree_paints_every_pane() {
        use ratatui::style::Color;
        fn gfx() -> WinNode {
            let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
            WinNode::Graphics(crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false })
        }
        fn buf(bg: u32, primary: bool) -> WinNode {
            WinNode::Buffer(BufferWindow { win: 0, lines: vec![], runs: vec![], para: vec![], images: vec![], scroll: 0, primary, bg: Some(bg), fg: None, panel: false, px_runs: Vec::new(), reads_input: false })
        }
        fn grid(bg: u32) -> WinNode {
            let mut g = GridWindow::default();
            g.resize(1, 1);
            g.active_rows = 1;
            g.bg = Some(bg);
            WinNode::Grid(g)
        }
        fn pair(vertical: bool, split: u16, first: WinNode, second: WinNode) -> WinNode {
            WinNode::Pair { vertical, split: Split { fixed: split }, border: true, key_bg: None, key_fg: None, first: Box::new(first), second: Box::new(second) }
        }
        let root =
            pair(false, 123,
                pair(false, 121,
                    pair(false, 36,
                        pair(false, 1, gfx(),
                            pair(false, 32,
                                pair(true, 58,
                                    pair(true, 1, buf(0xDDDDDD, false),        // buf75 header @(2,0)
                                        pair(true, 1, gfx(), buf(0xEEEEEE, false))), // buf67 body @(2,4)
                                    gfx()),
                                gfx())),
                        pair(true, 1, grid(0xDDDDDD),                          // grid79 @(37,0)
                            pair(true, 1, gfx(), buf(0xFFFFFF, true)))),        // buf4 main @(37,4)
                    gfx()),
                pair(false, 39,
                    pair(true, 1, buf(0xDDDDDD, false),                        // buf53 @(124,0)
                        pair(true, 1, gfx(),
                            pair(true, 42,
                                pair(true, 40, buf(0xEEEEEE, false), gfx()),   // buf47 @(124,4)
                                pair(true, 11,
                                    pair(true, 1, buf(0xDDDDDD, false),        // buf63 @(124,47)
                                        pair(true, 1, gfx(), buf(0xEEEEEE, false))), // buf57 @(124,51)
                                    gfx())))),
                    gfx()));
        let model = ScreenModel {
            root,
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0xFFFFFF)),
            fg: 0,
            content_size: (165, 60),
        };
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.theme = theme_with_bg_overrides(&[
            ("transcript", Color::Rgb(9, 9, 9)), // sentinel: an unpainted pane shows this
            ("upper_window", Color::Rgb(9, 9, 9)),
        ]);
        // Every pair in this tree is `border: true`, and since SQ-0821 that only
        // reserves a gutter when the THEME asks for a rule. This test is about panes
        // being painted, not about border policy, so it keeps the bordered geometry.
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single);
        let mut state = AppState::default();
        state.colors = colors;
        state.config.honor_game_colours = true;
        let area = Rect::new(0, 0, 165, 60);
        let mut b = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut b);

        let bgc = |x: u16, y: u16| b.cell((x, y)).unwrap().style().bg;
        // Each pane paints its own Normal bg at its dumped rect (not the sentinel).
        assert_eq!(bgc(3, 10), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "left panel body buf67 @(2,4)");
        assert_eq!(bgc(3, 0), Some(Color::Rgb(0xDD, 0xDD, 0xDD)), "left panel header buf75 @(2,0)");
        assert_eq!(bgc(60, 20), Some(Color::Rgb(0xFF, 0xFF, 0xFF)), "main window buf4 @(37,4)");
        assert_eq!(bgc(130, 20), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "right panel buf47 @(124,4)");
        assert_eq!(bgc(130, 53), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "lower-right buf57 @(124,51)");
        assert_eq!(bgc(130, 47), Some(Color::Rgb(0xDD, 0xDD, 0xDD)), "lower-right header buf63 @(124,47)");
        // No text pane left showing the sentinel (every pane painted).
        assert_ne!(bgc(3, 10), Some(Color::Rgb(9, 9, 9)));
    }

    /// SQ-0325 follow-up: the between-siblings separator is drawn in the split's
    /// KEY window colour — `key_fg` on `key_bg` — rather than the plain theme
    /// border style. Each channel falls back to the theme when `None`.
    #[test]
    fn separator_adopts_key_window_colour() {
        use ratatui::style::Color;
        let colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        // Vertical pair → horizontal rule; key fg red (0xFF0000), key bg blue (0x0000FF).
        draw_window_separator(
            area,
            true,
            crate::render::paneframe::BorderStyle::Single,
            Some(0x00FF_0000),
            Some(0x0000_00FF),
            &colors,
            &mut buf,
        );
        let c = buf.cell((2, 0)).unwrap();
        assert_eq!(c.style().fg, Some(Color::Rgb(0xFF, 0, 0)), "rule fg is the key window fg");
        assert_eq!(c.style().bg, Some(Color::Rgb(0, 0, 0xFF)), "rule bg is the key window bg");
        assert_eq!(c.symbol(), "\u{2500}", "vertical pair draws a horizontal rule glyph");
    }

    /// The Scott split: a `panel: true` buffer over the primary transcript draws
    /// with the themed `room_panel` colour, distinct from the transcript colour, so
    /// the top and bottom regions read apart.
    #[test]
    fn room_panel_draws_with_room_panel_theme() {
        use ratatui::style::Color;
        let mut panel = inline_buffer("I'm in a forest");
        panel.panel = true;
        let root = WinNode::Pair {
            vertical: true,
            split: Split { fixed: 1 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Buffer(panel)),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        };
        let model = ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) };

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.theme = theme_with_bg_overrides(&[
            ("transcript", Color::Rgb(9, 9, 9)),
            ("room_panel", Color::Rgb(0, 0, 128)),
        ]);
        let mut state = AppState::default();
        state.colors = colors;
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Top row (panel) uses room_panel bg, distinct from the transcript region
        // below it.
        let bgc = |x: u16, y: u16| buf.cell((x, y)).unwrap().style().bg;
        assert_eq!(bgc(0, 0), Some(Color::Rgb(0, 0, 128)), "panel uses room_panel bg");
        assert_ne!(bgc(0, 0), bgc(0, 3), "panel and transcript regions read apart");
    }

    #[test]
    fn is_simple_classifies_trees() {
        // Z-machine shape: grid over a (non-primary) buffer.
        let zm = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(GridWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&zm));
        // Lone buffer: simple.
        let lone = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&lone));
        // Two buffers: not simple.
        let two = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 10 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(BufferWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&two));
    }

    /// SQ-0325: a grid split BESIDE the buffer (winmethod_Left/Right, a horizontal
    /// pair) must NOT be the simple path. The simple path always draws the grid as a
    /// full-width top status band over the transcript, so a side-by-side grid would
    /// be mis-rendered as a centered top bar with the buffer full-width below
    /// ("the window is centered and we lose the main window"). It must take the
    /// generic path, which honours the left/right geometry.
    #[test]
    fn grid_beside_buffer_is_not_simple() {
        let side = ScreenModel {
            root: WinNode::Pair {
                vertical: false, // horizontal pair = Left/Right split
                split: Split { fixed: 20 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(GridWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&side), "a grid beside the buffer must use the generic path");
    }

    /// SQ-0325: a grid split BELOW the buffer (winmethod_Below → buffer-above-grid,
    /// a vertical pair with the buffer first) is likewise not the simple shape —
    /// the simple path would still draw the grid on TOP, in the wrong place.
    #[test]
    fn buffer_above_grid_is_not_simple() {
        let below = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 22 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
                second: Box::new(WinNode::Grid(GridWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&below), "a grid below the buffer must use the generic path");
    }

    /// SQ-0325 end-to-end: a text grid opened to the LEFT of the main buffer renders
    /// as a full-height left column (its cells filling that column from the top-left),
    /// NOT centered on the top row. Regression guard for the mis-routing.
    #[test]
    fn left_grid_renders_in_left_column_not_top_bar() {
        // A 6-col grid whose row 0 reads "GRID" (filling from the left), split to the
        // left of the primary buffer at a 6-col boundary in a 20-wide pane.
        let mut grid = GridWindow::default();
        grid.resize(4, 6); // 4 rows, 6 cols — a full window, not a 1-row status line
        for (i, ch) in "GRID".chars().enumerate() {
            grid.put(1, i as u16 + 1, ch, 0);
        }
        grid.active_rows = 4;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 6 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // "GRID" fills the left column from column 0 on row 0 — not centered, not a
        // top status bar over a full-width transcript.
        assert_eq!(row_text(&buf, 0, 6), "GRID  ", "grid fills the left column: {:?}", row_text(&buf, 0, 6));
    }

    #[test]
    fn split_area_bordered_vertical_and_horizontal() {
        let area = Rect::new(0, 0, 20, 10);
        // Borderless (b=0): the gutter is empty, children abut.
        let (top, sep, bottom) = split_area_bordered(area, true, 3, 0);
        assert_eq!(top, Rect::new(0, 0, 20, 3));
        assert_eq!(sep, Rect::new(0, 3, 20, 0));
        assert_eq!(bottom, Rect::new(0, 3, 20, 7));
        let (left, sep, right) = split_area_bordered(area, false, 8, 0);
        assert_eq!(left, Rect::new(0, 0, 8, 10));
        assert_eq!(sep, Rect::new(8, 0, 0, 10));
        assert_eq!(right, Rect::new(8, 0, 12, 10));
        // Bordered (b=1): a 1-cell gutter is carved out between the children.
        let (top, sep, bottom) = split_area_bordered(area, true, 3, 1);
        assert_eq!(top, Rect::new(0, 0, 20, 3));
        assert_eq!(sep, Rect::new(0, 3, 20, 1));
        assert_eq!(bottom, Rect::new(0, 4, 20, 6));
        let (left, sep, right) = split_area_bordered(area, false, 8, 1);
        assert_eq!(left, Rect::new(0, 0, 8, 10));
        assert_eq!(sep, Rect::new(8, 0, 1, 10));
        assert_eq!(right, Rect::new(9, 0, 11, 10));
        // Oversized fixed clamps to the extent; the border can't overflow either.
        let (l2, sep, r2) = split_area_bordered(area, true, 99, 1);
        assert_eq!(l2.height, 10);
        assert_eq!(sep.height, 0);
        assert_eq!(r2.height, 0);
    }

    #[test]
    fn generic_renders_grid_and_two_inline_buffers_in_subrects() {
        // Grid (top row) over a left|right buffer split.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                // Grid border Unspecified + theme sides off → frameless (SQ-0286);
                // this test checks buffer subrects, not border chrome.
                first: Box::new(WinNode::Grid(grid_with("STATUS"))),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 10 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Buffer(inline_buffer("LEFT"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("RIGHT"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Grid "STATUS" drawn on the top row (centered in its 20-wide area).
        assert!(row_text(&buf, 0, 20).contains("STATUS"), "grid row: {:?}", row_text(&buf, 0, 20));
        // Row 1: LEFT buffer in cols [0,10), RIGHT buffer in cols [10,20).
        assert_eq!(row_text(&buf, 1, 4), "LEFT");
        let right = row_text(&buf, 1, 20);
        assert!(right[10..].contains("RIGHT"), "right buffer at col>=10: {:?}", right);
    }

    /// SQ-0303 Stage 2: in the game-managed multi-window (generic) path the app
    /// must NOT frame the grid or borrow rows — the game owns the layout and draws
    /// its own borders (Kerkerkruip renders its panel rules as graphics windows).
    /// The grid renders frameless at its exact 1-row rect, the buffer below starts
    /// at the grid's exact bottom (not +2), and the columns stay row-aligned — even
    /// when the grid carries an explicit `winmethod_Border` and the theme has every
    /// border side on. (Replaces the old SQ-0200 border-row-borrow behavior.)
    #[test]
    fn generic_grid_renders_frameless_without_borrowing_rows() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        // Kerkerkruip-shaped: a center column of an explicit-Border status grid
        // over an inline BODY buffer, beside a right column of a graphics rule
        // (the game's own separator) + an inline SIDE panel. The graphics leaf
        // forces the generic path.
        let mut grid = grid_with("ST");
        grid.border = BorderPref::Border; // explicit winmethod_Border
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 8 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Pair {
                    vertical: true,
                    split: Split { fixed: 1 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Grid(grid)),
                    second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
                }),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 1 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(graphics_node()),
                    second: Box::new(WinNode::Buffer(inline_buffer("SIDE"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&model));

        // Theme with EVERY border side on — the old code would have framed the grid
        // and borrowed 2 rows; the fix suppresses both on the generic path.
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Frameless: NO box-drawing glyph in the grid/body columns [0,8). (Col 8 is
        // the game's own graphics rule, which legitimately renders as a │ — SQ-0332.)
        for y in 0..10 {
            for x in 0..8 {
                let s = buf.cell((x, y)).unwrap().symbol();
                assert!(
                    !"┌┐└┘─│".contains(s),
                    "no frame glyph on the generic path, found {s:?} at ({x},{y})"
                );
            }
        }
        // The graphics rule column (col 8) DOES draw a thin │ rule (the game's divider).
        assert_eq!(buf.cell((8, 5)).unwrap().symbol(), "\u{2502}", "graphics window renders its own thin rule");
        // Grid "ST" sits frameless on row 0 (cols=2 centered in the 8-wide center
        // column: x_off=(8-2)/2=3).
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "S", "grid content on row 0, no top border");
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), "T");
        // No row borrowed: the BODY buffer starts at the grid's EXACT bottom (row 1),
        // not shoved to row 3 by a 2-row border-borrow.
        assert_eq!(row_text(&buf, 1, 4), "BODY", "buffer below starts at grid bottom (row 1), not +2");
        // Columns stay row-aligned: the SIDE panel's first line is on row 0, level
        // with the grid — the center column is not shifted down relative to it.
        let side = row_text(&buf, 0, 20);
        let side_tail: String = side.chars().skip(9).collect();
        assert!(side_tail.contains("SIDE"), "side panel level with grid on row 0: {side:?}");
    }

    /// SQ-0303 Stage 2 guard: the SIMPLE (Z-machine / lone-grid) path is unchanged
    /// — a `BorderPref::Border` grid over the primary buffer still draws its frame
    /// (via `draw_upper_window`), so Counterfeit Monkey's coloured status border
    /// (SQ-0267) is preserved. Only the generic path went frameless.
    #[test]
    fn simple_path_still_frames_a_bordered_grid() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let mut grid = grid_with("HI");
        grid.border = BorderPref::Border;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&model), "grid-over-primary-buffer is the simple path");

        // Theme sides OFF: BorderPref::Border still forces a fallback single frame.
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // uw_w = 2 + 2 borders = 4, centered in 20 → x_off = 8; top-left corner at
        // (8,0), content pushed inside the frame to row 1.
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "┌", "simple path still frames a Border grid");
        assert_eq!(buf.cell((9, 1)).unwrap().symbol(), "H", "content sits inside the frame");
    }

    /// SQ-0303: gvm snaps its working width down and leaves a blank margin, so the
    /// composite must clamp to `content_size` — the right-edge leaf keeps its own
    /// width instead of ballooning into the surplus, and the margin stays blank.
    #[test]
    fn generic_clamps_composite_to_content_size_leaving_margin_blank() {
        // Grid (top row) over a left|right buffer split, content 8 wide inside a
        // 12-wide render area → a 4-col snap-margin. Without the clamp the RIGHT
        // buffer (last right-spine leaf) would stretch to absorb cols 5..12.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("ST"))),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 4 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Buffer(inline_buffer("LEFT"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("RGHT"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (8, 6),
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The RIGHT buffer draws in the content box's right half [4,8), NOT stretched
        // to the pane's right edge.
        let row1 = row_text(&buf, 1, 12);
        assert!(row1[4..8].contains("RGHT"), "right buffer sits in cols 4..8: {:?}", row1);
        // The snap-margin (cols 8..12) is blank — no leaf stretched into it.
        assert_eq!(&row1[8..12], "    ", "snap-margin blank on row 1: {:?}", row1);
        // The margin is blank on every row (right strip is full-height).
        for y in 0..6 {
            let r = row_text(&buf, y, 12);
            assert_eq!(&r[8..12], "    ", "snap-margin blank on row {}: {:?}", y, r);
        }
    }

    #[test]
    fn inline_buffer_renders_styled_runs() {
        let mut b = inline_buffer("abCD");
        b.runs = vec![vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 4), "abCD");
        // 'C' (col 2) carries the bold modifier.
        assert!(buf.cell((2, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(!buf.cell((0, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn inline_buffer_pushes_text_below_image_band() {
        // lines a / <image> / b. With a picker present the image at index 1
        // expands into a multi-row band, pushing "b" below the row it occupies
        // when images are off. Halfblocks font is 10x20 px; a 16x48-px image at
        // width 10 fits to a 2x3-cell band, so "b" lands on row 1 + 3 = 4.
        let mut px = image::RgbaImage::new(16, 48);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 40, 60, 255]);
        }
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None,
        };
        let b = BufferWindow {
            win: 0,
            lines: vec!["a".to_string(), String::new(), "b".to_string()],
            runs: vec![Vec::new(), Vec::new(), Vec::new()],
            para: vec![crate::state::ParaFmt::default(); 3],
            images: vec![None, Some(dummy), None],
            scroll: 0,
            primary: false,
            bg: None,
            fg: None,
            panel: false,
            px_runs: Vec::new(),
            reads_input: false,
        };
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let area = Rect::new(0, 0, 10, 8);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 1), "a", "first text line stays on row 0");
        let b_row = (0..8).find(|&y| row_text(&buf, y, 1).starts_with('b'));
        assert_eq!(b_row, Some(4), "\"b\" pushed below the 3-row image band");
    }

    #[test]
    fn story_pane_fills_game_background() {
        use ratatui::style::Color;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // honor_game_colours defaults to true.
        let mut model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        model.bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(2)); // black
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);
        // A blank interior cell (the empty transcript body, not the bottom input
        // row) carries the game background (black). Retargeted for SQ-0532/A-F5:
        // the default palette now resolves Standard colours to their ZMSD §8.3.1
        // true-colour equivalents ("2 = black (true $0000)") rather than the named
        // ANSI colour, so black is the exact RGB (0,0,0).
        assert_eq!(buf.cell((0, 2)).unwrap().style().bg, Some(Color::Rgb(0, 0, 0)),
            "story pane blank cell painted with game background");
    }

    /// The Z-machine 2-node tree must render byte-identical through
    /// `render_story_pane` vs. the direct `draw_upper_window` + `render_transcript`
    /// path it replaces.
    #[test]
    fn zmachine_two_node_tree_is_byte_identical() {
        use zvm::cpu::exec::Machine;
        // A minimal v3 machine → its neutral 2-node screen model.
        let story = {
            // Minimal valid v3 header (mirrors the render-test fixtures).
            let mut buf = vec![0u8; 0x0800];
            buf[0x00] = 3;
            buf[0x04] = 0x00; buf[0x05] = 0x40; // high mem base
            buf[0x06] = 0x00; buf[0x07] = 0x40; // initial pc
            buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict
            buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table
            buf[0x0E] = 0x03; buf[0x0F] = 0x00; // globals
            buf[0x08] = 0x04; buf[0x09] = 0x00; // static base
            buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table
            buf[0x0081] = 4; // dict entry size
            buf[0x0040] = 0xba; // quit
            buf
        };
        let mem = zvm::memory::Memory::new(story).expect("minimal v3");
        let machine = Machine::new(mem);
        let model = crate::session::screen_model_from_machine(&machine);
        assert!(is_simple(&model), "Z-machine tree is the simple case");

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript("You are in a room.");
        let area = Rect::new(0, 0, 40, 12);

        // Path A: render_story_pane.
        let mut buf_a = Buffer::empty(area);
        let ma = render_story_pane(&model, false, None, &state, area, &mut buf_a);

        // Path B: the exact code render_story_pane replaced.
        let mut buf_b = Buffer::empty(area);
        let used = draw_upper_window(model.grid().unwrap(), false, &state.colors, area, &mut buf_b, state.config.honor_game_colours, &mut Vec::new());
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let t = render_transcript(&model.status, None, &state, tarea, &mut buf_b, None);

        assert_eq!(buf_a, buf_b, "the simple path must be byte-identical to the legacy path");
        // The metrics are the transcript's own, verbatim — including the viewport,
        // which is the rows it gave to PROSE, not the pane rect it was handed
        // (`tarea.height`, one more here: the v3 status line takes a row). SQ-0823.
        assert_eq!((ma.scrollbar, ma.max_scroll, ma.viewport_rows), (t.scrollbar, t.max_scroll, t.viewport_rows));
        assert_eq!(ma.viewport_rows, tarea.height - 1, "the status line's row is not a readable transcript row");
    }

    fn graphics_node() -> WinNode {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 1,
            canvas: std::sync::Arc::new(img),
            version: 1,
            upscale: false,
        })
    }

    fn model_with(root: WinNode) -> ScreenModel {
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) }
    }

    /// The state `dialog_bounds` reads: the theme it resolves separators through,
    /// and the machine's `v6_text` — which supplies both the cell
    /// `classify_windows` splits on and the face `native_extent` measures with. The
    /// default face is the 8x16 cell every non-Macintosh v6 press declares, which is
    /// the grid the composites below are built on.
    fn dialog_state() -> crate::state::AppState {
        let mut state = crate::state::AppState::default();
        state.colors = ColorScheme::terminal_default();
        state
    }

    #[test]
    fn dialog_bounds_returns_frame_when_no_graphics() {
        // A pure-text tree: no graphics → dialogs keep full-frame centering.
        let model = model_with(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }));
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, Rect::new(0, 0, 20, 12), frame, &dialog_state()), frame);
    }

    #[test]
    fn dialog_bounds_excludes_left_graphics_sidebar_and_spans_map() {
        // Story pane (cols 0..20) = graphics sidebar (cols 0..10) | text buffer
        // (cols 10..20); the map occupies cols 20..40 of the frame. The dialog
        // region must be everything right of the graphics — text + map.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let story_area = Rect::new(0, 0, 20, 12);
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, story_area, frame, &dialog_state()), Rect::new(10, 0, 30, 12));
    }

    #[test]
    fn dialog_bounds_excludes_top_graphics_band() {
        // Graphics banner (rows 0..3) over the text buffer; no map (TranscriptFull).
        let model = model_with(WinNode::Pair {
            vertical: true,
            split: Split { fixed: 3 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let area = Rect::new(0, 0, 20, 12);
        assert_eq!(dialog_bounds(&model, area, area, &dialog_state()), Rect::new(0, 3, 20, 9));
    }

    #[test]
    fn dialog_bounds_ignores_graphics_when_story_pane_hidden() {
        // Story pane isn't laid out (empty), so graphics aren't on screen
        // and the dialog centers over the whole frame.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, Rect::default(), frame, &dialog_state()), frame);
    }

    /// A v6 composite: `art` at the given native pixel box, plus a primary story
    /// buffer at `story_px`. Cell rects are the native 8x16 quantization the
    /// session builds them with.
    fn v6_composite(art_px: (u16, u16, u16, u16), story_px: (u16, u16, u16, u16)) -> WinNode {
        let pw = |px: (u16, u16, u16, u16), node| crate::engine::PositionedWindow {
            x: px.0 / 8,
            y: px.1 / 16,
            w: px.2 / 8,
            h: px.3 / 16,
            x_px: px.0,
            y_px: px.1,
            w_px: px.2,
            h_px: px.3,
            left_margin: 0,
            right_margin: 0,
            node,
        };
        WinNode::Layered(vec![
            pw(art_px, graphics_node()),
            pw(story_px, WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        ])
    }

    #[test]
    fn dialog_bounds_ignores_a_v6_composites_spanning_art() {
        // SQ-1092: a `Layered` root is the graphical Z-machine, and a modal forces that
        // frame onto the CELL path (`!any_modal_overlay_open()`), which draws no frame
        // art at all. Subtracting it anyway put every modal in whatever strip it left:
        // Zork Zero's border window (0, 0, 640, 400) cut an 82x34 frame — story pane
        // inset one cell — down to `(0, 26, 82, 8)`, so a fifteen-row leader panel was
        // clamped to eight and lost its buttons.
        let model = model_with(v6_composite((0, 0, 640, 400), (86, 78, 468, 320)));
        let frame = Rect::new(0, 0, 82, 34);
        let story_area = Rect::new(1, 1, 80, 31);
        assert_eq!(
            dialog_bounds(&model, story_area, frame, &dialog_state()),
            frame,
            "art that spans the story is not drawn on the cell path, so it excludes nothing"
        );
    }

    /// The two callers of `v6_layout::cell_path_side_columns`, pinned against each
    /// other at a pane width where the OLD code diverged (SQ-1092).
    ///
    /// A stand-in for Journey's Amiga floppy at 160 columns: the renderer places the
    /// illustration at pane-proportional columns 3..65, while the walk that measured
    /// in the game's own NATIVE cells excluded only 2..33 whatever the pane — so a
    /// modal centred in what was left began at column 33 and the terminal drew the
    /// rest of the canyon wall over it. At 82 columns the two bases coincide (both
    /// 2..33), which is why every case written at the reported terminal size passes
    /// either way.
    ///
    /// Asserted as a RELATION to what the renderer places, not as a pinned rect: the
    /// point is that the two cannot drift apart again. The measured numbers are named
    /// in the message so a change to the shared rule is still legible in a failure.
    #[test]
    fn a_v6_side_columns_exclusion_tracks_the_columns_the_cell_path_places() {
        let items = match v6_composite((8, 16, 248, 272), (264, 16, 368, 272)) {
            WinNode::Layered(items) => items,
            other => panic!("expected a composite, got {other:?}"),
        };
        let state = dialog_state();
        let layout = crate::render::v6_layout::classify_windows(&items, state.v6_text.cell());
        let (native_w, _) = crate::render::v6_layout::native_extent(&items, &state.v6_text);
        // This two-window stand-in's unit screen is its story box's right edge; the
        // real frame has further windows and reaches 640. Either way the columns
        // below are proportional to it, which is the property under test.
        assert_eq!(native_w, 632, "the synthetic composite's unit screen");
        let model = model_with(WinNode::Layered(items.clone()));

        for (pane_w, want_cols) in [(82u16, (2u16, 33u16)), (160, (3, 65))] {
            let frame = Rect::new(0, 0, pane_w, 34);
            let story_area = Rect::new(1, 1, pane_w - 2, 31);
            let cols = crate::render::v6_layout::cell_path_side_columns(&layout, story_area, native_w);
            assert_eq!(cols.len(), 1, "one illustration column at {pane_w} columns");
            let drawn = (cols[0].x, cols[0].x + cols[0].w);
            assert_eq!(
                drawn, want_cols,
                "the cell path places Journey's illustration at these pane columns at {pane_w}"
            );
            let bounds = dialog_bounds(&model, story_area, frame, &state);
            assert!(
                bounds.x >= drawn.1,
                "at {pane_w} columns the dialog area must start at or right of the DRAWN column \
                 {drawn:?}, got {bounds:?} — the two measuring bases have drifted apart again"
            );
            assert_eq!(bounds.height, frame.height, "…and the dialog still gets the pane's full height");
        }
    }

    #[test]
    fn dialog_bounds_still_avoids_a_v6_side_column() {
        // The other half of SQ-1092, and the reason that arm filters rather than
        // returns: a chrome graphics window entirely BESIDE the story IS placed through
        // the image protocol on the cell path ("story content, not frame art"), so a
        // dialog must still keep clear of it. Journey's Amiga floppy (release 30) at
        // its gameplay frame: illustration at native x 8..256, story box at 264.
        let model = model_with(v6_composite((8, 16, 248, 272), (264, 16, 368, 272)));
        let frame = Rect::new(0, 0, 82, 34);
        let story_area = Rect::new(1, 1, 80, 31);
        let bounds = dialog_bounds(&model, story_area, frame, &dialog_state());
        assert_ne!(bounds, frame, "the illustration column is still excluded");
        assert!(bounds.x >= 33, "the dialog area starts right of the column, got {bounds:?}");
        assert_eq!(bounds.height, frame.height, "…and keeps the full height, so no modal is clipped");
    }

    #[test]
    fn graphics_leaf_renders_pixels() {
        use ratatui::layout::Rect;
        use ratatui::buffer::Buffer;
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 50, 50, 255]));
        let gw = crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let picker = ratatui_image::picker::Picker::halfblocks();
        let mut gr = crate::render::graphics::GraphicsRender::default();
        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        let style = ratatui::style::Style::default();
        gr.render(&picker, &gw, area, style, &mut buf);
        let has_pixels = (area.top()..area.bottom()).any(|y| (area.left()..area.right())
            .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}")));
        assert!(has_pixels, "graphics canvas should render half-block pixels");
    }

    /// SQ-0325: Counterfeit Monkey is a real Glulx layout (nonzero `content_size`),
    /// so it now routes through the GENERIC tree path — "compliant all the way",
    /// off the simple grid-over-transcript box and onto the spec separator/geometry.
    /// (This flips the old SQ-0303 premise, which kept CM on the simple path to
    /// preserve its framed status border; the generic path renders the game's true
    /// layout instead.) Its tree is still a status grid over the primary buffer — a
    /// vertical Pair with the grid first — but the nonzero extent forces the generic
    /// path. Skips when the (git-ignored) gblorb is absent.
    #[test]
    fn counterfeit_monkey_uses_the_generic_tree_path() {
        use crate::engine::Engine;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/CounterfeitMonkey-11.gblorb");
        if !path.exists() {
            eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent");
            return;
        }
        let blorb = blorb::Blorb::parse(std::fs::read(&path).unwrap()).expect("parse gblorb");
        let image = blorb.executable().expect("exec chunk").1.to_vec();
        let sess = crate::glulx_session::GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[])
            .expect("boot CM");
        let model = sess.screen();
        let (grids, buffers, others) = count_leaves(&model.root);
        assert!(
            !is_simple(&model),
            "CM is a real Glulx layout → generic path (grids={grids}, buffers={buffers}, others={others})"
        );
        // The shape is still a status grid stacked over the primary buffer.
        assert!(
            matches!(&model.root, WinNode::Pair { vertical: true, first, .. } if matches!(first.as_ref(), WinNode::Grid(_))),
            "CM tree is a vertical Pair with the status grid first"
        );
    }

    /// Build the theme used by the separator tests: every app-frame border off, so
    /// the only box-drawing glyphs in the pane are the inter-window separator rules.
    fn frameless_state() -> AppState {
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;
        state
    }

    /// The theme the separator tests need since SQ-0821: presence is the THEME's
    /// call, so a scheme with every border off draws no inter-window rule either.
    /// These tests are about the rule, so they ask for one.
    fn separator_state() -> AppState {
        let mut state = frameless_state();
        state.colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single);
        state
    }

    /// SQ-0325: a bordered STACKED pair (grid above an inline buffer, `border: true`,
    /// nonzero `content_size` → generic path) draws a horizontal `─` rule filling the
    /// gutter row between the two children, in the themed border colour; the grid sits
    /// above it and the buffer below.
    #[test]
    fn vertical_bordered_pair_draws_horizontal_rule() {
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("STATUS"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        assert!(!is_simple(&model));

        let state = separator_state();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The gutter row (row 1) between grid (row 0) and buffer (rows 2..) is all `─`.
        assert_eq!(row_text(&buf, 1, 20), "─".repeat(20), "gutter row filled with horizontal rule");
        // In the themed border colour.
        assert_eq!(
            buf.cell((10, 1)).unwrap().style().fg,
            state.colors.theme.get("upper_window_border").style.fg,
            "separator carries the themed window-border colour"
        );
        // Grid content on row 0, buffer below the rule on row 2.
        assert!(row_text(&buf, 0, 20).contains("STATUS"), "grid above the rule: {:?}", row_text(&buf, 0, 20));
        assert_eq!(row_text(&buf, 2, 4), "BODY", "buffer below the rule");
    }

    /// SQ-0325: a bordered SIDE-BY-SIDE pair (grid left of the primary buffer,
    /// `border: true`) draws a vertical `│` rule filling the gutter column between
    /// the children, in the themed border colour; the grid sits left of it.
    #[test]
    fn horizontal_bordered_pair_draws_vertical_rule() {
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 6 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("GRID"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        assert!(!is_simple(&model));

        let state = separator_state();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The gutter column (col 6, after the 6-wide grid) is all `│` on every row.
        for y in 0..6 {
            assert_eq!(buf.cell((6, y)).unwrap().symbol(), "│", "vertical rule at split col, row {y}");
        }
        assert_eq!(
            buf.cell((6, 0)).unwrap().style().fg,
            state.colors.theme.get("upper_window_border").style.fg,
            "separator carries the themed window-border colour"
        );
        // Grid content left of the rule (cols < 6) on row 0.
        assert!(row_text(&buf, 0, 6).contains("GRID"), "grid left of the rule: {:?}", row_text(&buf, 0, 6));
    }

    /// SQ-0325: `border: false` on the same shapes draws NO separator glyph — the
    /// children abut with no gutter. Guards that the rule is gated on the flag.
    #[test]
    fn unbordered_pairs_draw_no_separator() {
        for vertical in [true, false] {
            let model = ScreenModel {
                root: WinNode::Pair {
                    vertical,
                    split: Split { fixed: if vertical { 1 } else { 6 } },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Grid(grid_with("GRID"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
                },
                status: StatusModel::HostManaged,
                bg: 0,
                fg: 0,
                content_size: (20, 6),
            };
            let state = frameless_state();
            let area = Rect::new(0, 0, 20, 6);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            for y in 0..6 {
                for x in 0..20 {
                    let s = buf.cell((x, y)).unwrap().symbol();
                    assert!(
                        !"─│".contains(s),
                        "no separator glyph when border:false (vertical={vertical}), found {s:?} at ({x},{y})"
                    );
                }
            }
        }
    }

    /// SQ-0821: the SHIPPED default draws no inter-window rule, and reserves no
    /// gutter for one either.
    ///
    /// Glk's `winmethod_Border` is the DEFAULT value of that flag rather than a
    /// considered request, so honouring it put a rule under the status bar of
    /// essentially every Glulx game whose author never thought about borders. The
    /// theme decides now, and `upper_window_border`'s style defaults to `none`.
    ///
    /// The gutter half matters as much as the glyph: a reserved-but-blank row is
    /// still a gap between the status bar and the story.
    #[test]
    fn the_default_theme_draws_no_separator_and_reserves_no_gutter() {
        for vertical in [true, false] {
            let model = ScreenModel {
                root: WinNode::Pair {
                    vertical,
                    split: Split { fixed: if vertical { 1 } else { 6 } },
                    // The game asks for a border, the way almost every Glk game does
                    // simply by not asking for `winmethod_NoBorder`.
                    border: true,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Grid(grid_with("GRID"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
                },
                status: StatusModel::HostManaged,
                bg: 0,
                fg: 0,
                content_size: (20, 6),
            };
            // A stock terminal-default scheme: no style.toml, no border asked for.
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            let area = Rect::new(0, 0, 20, 6);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);

            for y in 0..6 {
                for x in 0..20 {
                    let s = buf.cell((x, y)).unwrap().symbol();
                    assert!(
                        !"─│═║━┃".contains(s),
                        "the default theme draws no rule (vertical={vertical}), found {s:?} at ({x},{y})"
                    );
                }
            }
            // …and no gutter: the second child starts where the first ends.
            if vertical {
                assert_eq!(row_text(&buf, 1, 4), "BODY", "no gutter row between the children");
            } else {
                assert_eq!(&row_text(&buf, 0, 10)[6..10], "BODY", "no gutter column between them");
            }
        }
    }

    /// SQ-0821: `upper_window_border`'s STYLE reaches the separator's glyph.
    ///
    /// It used to draw a hard-coded `─`/`│`, so `style = "double"` in `style.toml`
    /// parsed, landed on `upper_window_border_sides`, and was read by the status
    /// frame and by nothing else — the setting appeared to do nothing at all, which
    /// is what the user reported. A per-side glyph override still beats the style.
    #[test]
    fn the_separator_glyph_follows_the_themed_border_style_and_any_override() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let model = |vertical: bool| ScreenModel {
            root: WinNode::Pair {
                vertical,
                split: Split { fixed: if vertical { 1 } else { 6 } },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("GRID"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        let rule_cell = |state: &AppState, vertical: bool| -> String {
            let area = Rect::new(0, 0, 20, 6);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model(vertical), false, None, state, area, &mut buf);
            let at = if vertical { (10, 1) } else { (6, 3) };
            buf.cell(at).unwrap().symbol().to_string()
        };

        for (style, horizontal, vertical_glyph) in [
            (BorderStyle::Single, "─", "│"),
            (BorderStyle::Double, "═", "║"),
            (BorderStyle::Thick, "━", "┃"),
        ] {
            let mut state = separator_state();
            state.colors.upper_window_border_sides = PaneSides::all(style);
            assert_eq!(rule_cell(&state, true), horizontal, "{style:?}: stacked pair's horizontal rule");
            assert_eq!(rule_cell(&state, false), vertical_glyph, "{style:?}: side-by-side pair's vertical rule");
        }

        // An explicit glyph still wins over the style's own.
        let mut state = separator_state();
        state.colors.upper_window_border_sides = PaneSides::all(BorderStyle::Double);
        state.colors.upper_window_border_glyphs.top = Some("=".to_string());
        assert_eq!(rule_cell(&state, true), "=", "glyph_top beats the style's ═");
    }

    /// SQ-0821: the split's key-window colours only speak when the player has asked
    /// for the game's colours.
    ///
    /// They used to override the themed border colour unconditionally — so with
    /// `honor_game_colours = false`, which is precisely the setting that says
    /// "ignore what the game wants", a styled border colour was still overridden by
    /// the game. That was the third reason a `style.toml` border appeared not to
    /// work, after presence and glyph.
    #[test]
    fn the_key_window_colour_is_ignored_when_game_colours_are_declined() {
        use ratatui::style::Color;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: true,
                key_bg: Some(0x0000_00FF),
                key_fg: Some(0x00FF_0000),
                first: Box::new(WinNode::Grid(grid_with("GRID"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        let rule_fg = |honor: bool| {
            let mut state = separator_state();
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, 20, 6);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            (buf.cell((10, 1)).unwrap().style().fg, state.colors.theme.get("upper_window_border").style.fg)
        };

        let (fg_on, _) = rule_fg(true);
        assert_eq!(fg_on, Some(Color::Rgb(0xFF, 0, 0)), "honoured: the key window's red wins");

        let (fg_off, themed) = rule_fg(false);
        assert_eq!(fg_off, themed, "declined: the theme's own border colour is authoritative");
        assert_ne!(fg_off, Some(Color::Rgb(0xFF, 0, 0)), "…and the game's red does not reach it");
    }

    #[test]
    fn empty_graphics_neighbour_draws_separator_painted_one_suppresses() {
        // narco frames its story with graphics windows it never paints; the frame
        // must still get our separator rule. Kerkerkruip PAINTS its dividers, so
        // those still suppress our rule (no doubling). (SQ-0340, refines SQ-0332)
        let empty_graphics = || {
            let img = image::RgbaImage::new(9, 57); // opened but never drawn → transparent
            WinNode::Graphics(crate::engine::GraphicsWindow { win: 4, canvas: std::sync::Arc::new(img), version: 1, upscale: false })
        };
        let make = |second: WinNode| ScreenModel {
            root: WinNode::Pair {
                vertical: false, // left/right split → a │ separator
                split: Split { fixed: 10 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(inline_buffer("STORY"))),
                second: Box::new(second),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        let state = separator_state();
        let area = Rect::new(0, 0, 20, 6);
        let has_rule = |m: &ScreenModel| {
            let mut buf = Buffer::empty(area);
            render_story_pane(m, false, None, &state, area, &mut buf);
            (0..6).any(|y| (0..20).any(|x| buf.cell((x, y)).unwrap().symbol() == "\u{2502}"))
        };
        assert!(has_rule(&make(empty_graphics())), "empty graphics neighbour → our separator drawn");
        assert!(!has_rule(&make(graphics_node())), "painted graphics divider → our separator suppressed");
    }

    #[test]
    fn collect_graphics_ids_finds_every_graphics_leaf() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let other = WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(img),
            version: 1,
            upscale: false,
        });
        let tree = WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()), // win: 1
            second: Box::new(other),
        };
        let mut ids = std::collections::HashSet::new();
        collect_graphics_ids(&tree, &mut ids);
        assert_eq!(ids, std::collections::HashSet::from([1, 7]));
    }

    /// SQ-0639: v6 painted text is UNTRUSTED game text and it can carry control
    /// characters — `print_unicode` (ZMSD EXT:0x0B) prints any codepoint a story
    /// asks for, and a story-supplied Unicode translation table can map ZSCII 155+
    /// to one. Handing such a run straight to `Buffer::set_stringn` does not draw a
    /// control char, it DROPS it — and every glyph after it shifts a column left,
    /// which for pixel-positioned v6 runs is exactly the alignment the path exists
    /// to preserve. Blanking to a space keeps the columns. Pinned in both
    /// `honor_game_colours` modes.
    #[test]
    fn painted_game_text_blanks_control_chars_instead_of_shifting_the_run() {
        let colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 20, 3);
        let read = |buf: &Buffer, y: u16, n: u16| -> String {
            (0..n).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        for honor in [true, false] {
            // A painted run with a BEL in the middle, at native (row 0, col 0).
            let t = crate::engine::PxText::derived(1, 1, "AB\u{7}CD".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT);
            let runs: Vec<&crate::engine::PxText> = vec![&t];
            let mut buf = Buffer::empty(area);
            draw_painted_screen(
                &runs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), TextInk::new(honor, &colors), &[], 640,
                zvm::screen::V6Cell::DEFAULT,
            );
            assert_eq!(read(&buf, 0, 5), "AB CD", "honor={honor}: the control char blanks, D stays in column 4");

            // The anchored status band takes its groups from the same runs.
            let mut band = Buffer::empty(area);
            assert!(place_anchored_row(
                &mut band, area, 0, "L\u{1}T", "", "R\u{2}T", ratatui::style::Style::default()
            ));
            assert_eq!(read(&band, 0, 3), "L T", "honor={honor}: LEFT group keeps its width");
            assert_eq!(read(&band, 0, area.width), format!("{:<17}R T", "L T"), "honor={honor}: RIGHT stays flush right");
        }
    }

    #[test]
    fn collect_graphics_ids_walks_a_layered_composite() {
        // SQ-0637: a v6 composite is a `Layered` root. Missing that arm reported NO
        // live windows for such a frame, so `retain_live` cleared the whole protocol
        // cache every frame — a full re-encode, and under kitty a fresh id per frame
        // with the previous ones never deleted. The id walk must reach every graphics
        // leaf of a composite.
        //
        // This used to assert PARITY with `collect_graphics_rects`, and SQ-1092 ended
        // that: the two walks answer different questions and now differ on exactly this
        // tree. Which windows are LIVE decides what the protocol cache keeps, and a
        // composite's are (the ring drew them last frame and will again the moment the
        // modal closes). Which rects a DIALOG must avoid is asked only while a modal is
        // up, and a modal is what takes the composite off the screen — see
        // `collect_graphics_rects`'s `Layered` arm. Parity was a convenient proxy for
        // "don't forget the arm", never the requirement; the requirement is asserted
        // directly below.
        let pane = Rect::new(0, 0, 40, 20);
        let pw = |win: u32, x: u16, y: u16| PositionedWindow {
            x,
            y,
            w: 6,
            h: 4,
            x_px: x * 8,
            y_px: y * 16,
            w_px: 48,
            h_px: 64,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win,
                canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))),
                version: 1,
                upscale: false,
            }),
        };
        let text = PositionedWindow { node: WinNode::Buffer(inline_buffer("STORY")), ..pw(9, 0, 8) };
        let tree = WinNode::Layered(vec![pw(3, 0, 0), text, pw(5, 10, 2)]);

        let mut ids = std::collections::HashSet::new();
        collect_graphics_ids(&tree, &mut ids);
        assert_eq!(ids, std::collections::HashSet::from([3, 5]), "every layered graphics leaf is live");

        // …and the rect walk deliberately sees NEITHER of them (SQ-1092): win 3 overlaps
        // the story box, win 5 clears its rows entirely, and the cell path a modal
        // forces places only a column that is beside the story AND alongside its rows.
        let mut rects = Vec::new();
        collect_graphics_rects(&tree, pane, &mut rects, &dialog_state());
        assert!(rects.is_empty(), "neither window is a side column the cell path would place");
    }

    /// v6 layered composite (Phase 1b): a full-area solid graphics window
    /// (background) with a small grid (foreground) drawn on top. The grid's
    /// one non-blank cell must land at its absolute rect; a BLANK grid cell
    /// must leave the graphics layer's colour showing through — cell-text-wins.
    #[test]
    fn layered_composite_draws_zorder_with_cell_text_wins() {
        use ratatui::style::Color;

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // No picker: this test exercises the Phase 1b cell composite fallback.
        // With a picker, Phase 1c takes over `Layered` and draws one pixel image
        // instead (see `layered_composite_*` picker-path coverage elsewhere).

        // Background: a full-area solid-colour graphics window.
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]));
        let background = PositionedWindow {
            x: 0,
            y: 0,
            w: 10,
            h: 6,
            x_px: 0,
            y_px: 0,
            w_px: 80,
            h_px: 48,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 1,
                canvas: std::sync::Arc::new(img),
                version: 1,
                upscale: false,
            }),
        };

        // Foreground: a 3x2 grid, positioned at (2,2), with a single non-blank cell.
        let mut grid = GridWindow::default();
        grid.resize(2, 3);
        grid.active_rows = 2;
        grid.put(1, 1, 'X', 0);
        let foreground = PositionedWindow {
            x: 2,
            y: 2,
            w: 3,
            h: 2,
            x_px: 16,
            y_px: 16,
            w_px: 24,
            h_px: 16,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Grid(grid),
        };

        let model = ScreenModel {
            root: WinNode::Layered(vec![background, foreground]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (10, 6),
        };

        let area = Rect::new(0, 0, 10, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The grid's non-blank cell is drawn at its absolute rect (2,2).
        assert_eq!(buf.cell((2, 2)).unwrap().symbol(), "X", "grid glyph at its absolute cell");

        // A blank grid cell (grid col 2, row 1 → absolute (3,2)) is transparent:
        // the background graphics colour shows through instead of a grid fill.
        assert_eq!(
            buf.cell((3, 2)).unwrap().style().bg,
            Some(Color::Rgb(10, 20, 30)),
            "blank grid cell is transparent — graphics layer shows through"
        );
    }

    /// A synthetic v6 `Layered` model (native 320×200: a full-area opaque chrome
    /// graphics window + a primary story `Buffer` at Zork0's win0 box), for the
    /// Lane H hybrid-branch tests.
    fn hybrid_v6_model() -> ScreenModel {
        // Native-sized opaque chrome so build_chrome_canvas yields a real ring.
        let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        // Story: the primary buffer at the win0 box (43,39,234,160).
        let story = PositionedWindow {
            x: 5, y: 4, w: 29, h: 20, x_px: 43, y_px: 39, w_px: 234, h_px: 160,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![chrome, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn hybrid_deep_status_outside_story_box_keeps_the_ring() {
        // SQ-0494: Arthur paints its status bar as reverse px_text runs at a deep
        // native row (12 on the real 640×400 screen) ABOVE its story buffer — with
        // graphics windows carrying the top image panel and side borders. That is
        // ordinary gameplay chrome, NOT a menu takeover: the ring path must be
        // kept (the status text belongs to the pixel ring, so it must NOT be
        // painted into the terminal cells the way a routed menu screen is).
        //
        // SQ-0944 split this by backend, and the split is the point rather than an
        // accommodation. "Did the run reach a cell?" was only ever a PROXY for "was
        // the ring path taken", and on half-blocks the ring itself now stamps text
        // that sits on artwork as glyphs — same path, same owner, different medium.
        // So the proxy is asserted where it still discriminates (kitty, which is
        // what Arthur's real case runs on) and inverted where the new behaviour
        // says it must appear; `metrics.is_some()` asserts the ring path directly
        // on both, and a menu takeover would fail that on either.
        for protocol in [
            ratatui_image::picker::ProtocolType::Kitty,
            ratatui_image::picker::ProtocolType::Halfblocks,
        ] {
        let glyphs_over_art = protocol == ratatui_image::picker::ProtocolType::Halfblocks;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some({
            // `halfblocks()` then override: the only non-deprecated constructor
            // that does not need a live terminal to query.
            let mut p = ratatui_image::picker::Picker::halfblocks();
            p.set_protocol_type(protocol);
            p
        });
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("HELLO STORY WORLD");

        let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        // Status grid: a non-blank run at native row 6 (deep, ≥ STATUS_BAND_ROWS)
        // but ABOVE the story buffer, which starts at row 7 (y_px 112).
        let status = PositionedWindow {
            x: 0, y: 6, w: 40, h: 1, x_px: 0, y_px: 96, w_px: 320, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                win: 0,
                fill: None,
                cols: 40, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![crate::engine::PxText::derived(97, 1, "Score: 0".into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT)],
            }),
        };
        let story = PositionedWindow {
            x: 0, y: 7, w: 40, h: 10, x_px: 0, y_px: 112, w_px: 320, h_px: 80,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let model = ScreenModel {
            root: WinNode::Layered(vec![chrome, status, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        };
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let metrics = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        assert!(metrics.is_some(), "ring path taken (it returns inset metrics)");
        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect::<String>() + "\n")
            .collect();
        assert_eq!(
            screen.contains("Score: 0"),
            glyphs_over_art,
            "{protocol:?}: the deep-but-outside-story status belongs to the ring either way — \
             rasterised into its band where a glyph cannot sit over a placement, stamped as \
             glyphs where one can, and never routed into cells as a menu takeover:\n{screen}"
        );
        }
    }

    #[test]
    fn hybrid_renders_story_as_terminal_text_in_an_inset_viewport() {
        // Hybrid + a picker: the Layered arm draws the chrome ring and renders the
        // story window as REAL terminal text (via render_transcript) into an inset
        // viewport — so render_node returns Some(metrics) and the transcript
        // publishes its geometry inside (strictly smaller than) the full pane.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("HELLO STORY WORLD");

        let model = hybrid_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        let m = m.expect("hybrid story viewport returns primary-buffer metrics");
        assert!(m.viewport_rows > 0, "story viewport has rows");

        // The transcript rendered as terminal cells into an inset viewport.
        let geom = state.transcript_geom.get().expect("hybrid renders the transcript as terminal cells");
        let vp = geom.area;
        assert!(vp.width < area.width && vp.height < area.height, "viewport is inset inside the chrome ring: {vp:?}");
        assert!(vp.x >= area.x && vp.y >= area.y && vp.right() <= area.right() && vp.bottom() <= area.bottom(),
            "viewport stays inside the pane: {vp:?}");
    }

    #[test]
    fn hybrid_menu_screen_renders_coherent_all_text_with_transcript() {
        // SQ-0484: Shogun's boot menu keeps window 0 (the story buffer) open AND
        // paints its three menu items as DEEP chrome runs (native rows ≥
        // STATUS_BAND_ROWS). The old ring+viewport path split that menu across the
        // raster pixel ring (items mapping above the terminal viewport → rendered
        // as pixel art) and the terminal overlay (items inside it → terminal text),
        // giving the mixed "first option raster, rest terminal text" screen. A menu
        // screen must instead route to the cell path — the story transcript plus the
        // menu painted over it as ONE coherent all-text screen, matching the
        // frameless path — so all three items are terminal cells and the transcript
        // ("You may choose to:") is preserved.
        //
        // SQ-0886 narrowed that routing to the case it is right for, which is this
        // one: a takeover screen with NO ARTWORK behind it. The chrome window here
        // is therefore transparent — a game that publishes a graphics window and
        // never draws in it, which is what advent.z6 is. When the game HAS painted
        // art the cell path throws it away, and the sibling case below is that.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("You may choose to:");

        let mut model = hybrid_v6_model();
        if let WinNode::Layered(items) = &mut model.root {
            if let WinNode::Graphics(g) = &mut items[0].node {
                g.canvas = std::sync::Arc::new(image::RgbaImage::new(320, 200));
            }
        }
        // A chrome grid whose pixel runs sit DEEP (native rows 8/9/10, ≥
        // STATUS_BAND_ROWS) inside the story box (native y 39..199 → the 8×16 cell
        // rows land at (y-1)/16). Distinct rows, like Shogun's real 21/22/23.
        let menu = PositionedWindow {
            x: 12, y: 8, w: 1, h: 3, x_px: 100, y_px: 129, w_px: 1, h_px: 48,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                win: 0,
                fill: None,
                cols: 1, rows: 3, cells: vec![], active_rows: 3, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText::derived(129, 101, "START the game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                    crate::engine::PxText::derived(145, 101, "RESTORE a saved game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                    crate::engine::PxText::derived(161, 101, "QUIT the game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                ],
            }),
        };
        if let WinNode::Layered(items) = &mut model.root {
            items.push(menu);
        }
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let _ = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        // A menu screen publishes a full terminal transcript geometry (the cell
        // path), NOT a raster/hybrid image — so the transcript renders as real cells.
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        let screen: String = (0..area.height).map(|y| row_text(y) + "\n").collect();
        // The transcript prompt is preserved (dropped by a painted-only path).
        assert!(screen.contains("You may choose to:"), "transcript prompt preserved, screen:\n{screen}");
        // All three items render as terminal text on their distinct deep rows,
        // placed relative to the STORY BOX they are painted inside (SQ-0697): this
        // model's story window starts at native y=39 — row 2 — with no chrome above
        // it, so the box takes the pane's top row and its contents come with it. The
        // items' native rows 8/9/10 therefore land two rows up, at 6/7/8; stamping
        // them at absolute native rows instead would tear a menu away from the
        // transcript it is painted over.
        assert_eq!(row_text(6).trim(), "START the game", "row 6 is the START item, screen:\n{screen}");
        assert_eq!(row_text(7).trim(), "RESTORE a saved game", "row 7 is the RESTORE item");
        assert_eq!(row_text(8).trim(), "QUIT the game", "row 8 is the QUIT item");
    }

    #[test]
    fn hybrid_menu_screen_over_artwork_takes_the_ring() {
        // SQ-0886: the same takeover screen, with the game's ARTWORK behind it —
        // Shogun's boot menu, whose credits and menu sit on the machine's own ground
        // between two ornate side panels. The cell path above draws no art at all, so
        // routing this screen there lost every pixel the game had drawn: no panels
        // anywhere and the story window's page flooded across the pane (`#000000` over
        // 761 of 800 columns, measured on the Amiga floppy AND the IBM Blorb).
        //
        // SQ-0886 sent it to the composite; SQ-0892 sends it to the RING, which draws
        // the panels as art AND the menu as glyphs — the composite can only draw the
        // menu as pixels, and SQ-0750 reserves raster for pixels the runs cannot
        // account for. Which destination it takes is the whole of this case; that the
        // menu SURVIVES the trip is the sibling assertion below it.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("You may choose to:");

        // `hybrid_v6_model` ships an OPAQUE chrome canvas, which is the whole
        // difference from the case above.
        let mut model = hybrid_v6_model();
        let menu = PositionedWindow {
            x: 12, y: 8, w: 1, h: 3, x_px: 100, y_px: 129, w_px: 1, h_px: 48,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                win: 0,
                fill: None,
                cols: 1, rows: 3, cells: vec![], active_rows: 3, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText::derived(129, 101, "START the game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                    crate::engine::PxText::derived(145, 101, "RESTORE a saved game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                    crate::engine::PxText::derived(161, 101, "QUIT the game".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                ],
            }),
        };
        if let WinNode::Layered(items) = &mut model.root {
            items.push(menu);
        }
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let _ = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
        assert_eq!(
            path, "hybrid-ring",
            "a menu takeover with the game's artwork behind it takes the RING — the cell path \
             draws no art, and the composite draws the menu as pixels (SQ-0892)"
        );

        // …and the menu arrives as GLYPHS, on the three consecutive rows the game
        // printed them on. This is the whole of SQ-0892 in one assertion: the runs
        // abut at 8px pitch and stand on consecutive 16px rows, so rounding each one
        // on its own axis-independently is what produced `SI(RT th e ga me` across
        // the columns and a skipped row down the middle of the menu.
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        let screen: String = (0..area.height).map(|y| row_text(y) + "\n").collect();
        let row_of = |s: &str| {
            (0..area.height)
                .find(|&y| row_text(y).contains(s))
                .unwrap_or_else(|| panic!("the menu item {s:?} reaches the pane as text:\n{screen}"))
        };
        let start = row_of("START the game");
        assert_eq!(
            (row_of("RESTORE a saved game"), row_of("QUIT the game")),
            (start + 1, start + 2),
            "the three items keep the game's own consecutive rows:\n{screen}"
        );
    }

    /// SQ-0896: art the game painted INSIDE its story window, on a frame that takes
    /// the ring today — the capability gap, and the one case that needs no routing
    /// change to demonstrate.
    ///
    /// Native 320x200. A chrome FRAME (win 7) painted only in the 20px border, so
    /// the ring has real content and `story_clear_native` finds nothing overlapping
    /// window 0. Window 0 is (40,40,240,120) — inset from every screen edge, so
    /// `story_covers_screen` is false and none of `picture_takeover_reason`'s arms fire.
    /// Inside it, a `win == 0` Graphics plate covering the LEFT HALF of the window.
    ///
    /// Before this quest the viewport was the raw window box: the transcript opened
    /// straight over the plate, and the plate reached the screen through nothing —
    /// `classify_windows` sets it aside as `story_gfx` so the chrome canvas never
    /// carries it, and `blit_story_gfx` was reachable from the RASTER path alone. The
    /// prose was drawn over art the player could not see.
    ///
    /// Now the viewport is cut from what the art LEAVES, so the plate's columns are
    /// outside it — and everything outside the viewport is the ring's, drawn by
    /// machinery that has not changed.
    #[derive(Clone, Copy, Debug)]
    enum PlateSide {
        Left,
        Top,
    }

    fn plate_in_story_window_model(with_plate: bool, side: PlateSide) -> ScreenModel {
        // A chrome frame: opaque 20px border, hollow middle. Not a backdrop — the
        // inset must find nothing to take off window 0.
        let mut frame = image::RgbaImage::new(320, 200);
        for (x, y, px) in frame.enumerate_pixels_mut() {
            if x < 20 || y < 20 || x >= 300 || y >= 180 {
                *px = image::Rgba([40, 30, 20, 255]);
            }
        }
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(frame), version: 1, upscale: false,
            }),
        };
        // The story window's OWN plate, in a colour nothing else on the screen uses:
        // either the left half of window 0 (which the ring carves off as a FLANK) or
        // its top half (a full-width TOP band). The two reach the screen through
        // different draw arms — a flank composes its own source image, a full-width
        // strip is a straight crop of the frame-shared scaled canvas — so both have
        // to be exercised or half the fix is untested.
        let (pw_px, ph_px) = match side {
            PlateSide::Left => (120u16, 120u16),
            PlateSide::Top => (240, 60),
        };
        let plate_img = image::RgbaImage::from_pixel(
            pw_px as u32, ph_px as u32, image::Rgba([200, 10, 120, 255]),
        );
        let plate = PositionedWindow {
            x: 5, y: 2, w: 15, h: 15, x_px: 40, y_px: 40, w_px: pw_px, h_px: ph_px,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 0, canvas: std::sync::Arc::new(plate_img), version: 1, upscale: false,
            }),
        };
        let story = PositionedWindow {
            x: 5, y: 2, w: 30, h: 15, x_px: 40, y_px: 40, w_px: 240, h_px: 120,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let mut items = vec![chrome];
        if with_plate {
            items.push(plate);
        }
        items.push(story);
        ScreenModel {
            root: WinNode::Layered(items),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    /// Render `plate_in_story_window_model` and report the story viewport's columns
    /// and whether the plate's ink reached the pane.
    fn plate_frame_probe(with_plate: bool, honor: bool, side: PlateSide) -> (Rect, bool) {
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        state.push_transcript("HELLO STORY WORLD");

        let model = plate_in_story_window_model(with_plate, side);
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let metrics = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );

        // Non-vacuity: this frame must actually reach the ring. If a future change to
        // `picture_takeover_reason` diverts it, every number below would describe a screen
        // the ring never drew.
        let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
        assert_eq!(path, "hybrid-ring", "plate={with_plate} honor={honor} {side:?}: the fixture is a RING frame");
        assert!(metrics.is_some(), "plate={with_plate} honor={honor} {side:?}: the ring rendered a transcript");

        let geom = state.transcript_geom.get().expect("the transcript has geometry");
        let plate_ink = image::Rgba([200u8, 10, 120, 255]);
        let painted = (0..area.height).any(|y| {
            (0..area.width).any(|x| {
                let c = buf.cell((x, y)).unwrap();
                [c.fg, c.bg].iter().any(|col| {
                    matches!(col, ratatui::style::Color::Rgb(r, g, b)
                        if *r == plate_ink[0] && *g == plate_ink[1] && *b == plate_ink[2])
                })
            })
        });
        (geom.area, painted)
    }

    /// Both `honor_game_colours` modes (CLAUDE.md): the plate is something the game
    /// DREW, so it belongs on the screen whether or not its palette is honoured, and
    /// a colour area pinned in one mode only has masked every game-colour regression
    /// this project has had.
    ///
    /// A/B on ONE frame rather than against a computed cell number: the same model
    /// with and without the plate, on each of the two edges. That is what makes the
    /// assertion mean what it says — the viewport moved BECAUSE of the plate — and it
    /// carries its own falsification, since reverting the ring's use of
    /// `story_text_native` makes the two viewports identical and leaves the plate
    /// unpainted.
    ///
    /// FALSIFIED, both halves, on the honor=true case:
    /// * viewport from the declared box instead of `story_text_native` →
    ///   `bare (5, 34) vs plated (5, 34)` — the plate makes no difference and the
    ///   transcript opens straight over it, which is the reported gap;
    /// * `blit_story_gfx` off the ring's band canvas → the TOP case loses its ink
    ///   ("the plate is drawn by the ring"), while the LEFT case survives, because a
    ///   flank composes its source from the art canvas and a full-width strip crops
    ///   the band canvas. That asymmetry is exactly why both sides are tested.
    fn plate_in_story_window_case(honor: bool, side: PlateSide) {
        let (bare, bare_painted) = plate_frame_probe(false, honor, side);
        let (plated, plate_painted) = plate_frame_probe(true, honor, side);

        assert!(!bare_painted, "honor={honor} {side:?}: nothing paints the plate's ink with no plate");
        match side {
            PlateSide::Left => {
                assert!(
                    plated.x > bare.x,
                    "honor={honor}: the story viewport starts at the plate's right edge, not \
                     the window's left edge — the text region is what the art LEAVES \
                     (SQ-0896). bare {bare:?} vs plated {plated:?}"
                );
                assert_eq!(
                    (plated.right(), plated.y, plated.bottom()),
                    (bare.right(), bare.y, bare.bottom()),
                    "honor={honor}: only the edge the plate stands on moves; nothing is \
                     rasterised that the art did not demand (SQ-0750). \
                     bare {bare:?} vs plated {plated:?}"
                );
            }
            PlateSide::Top => {
                assert!(
                    plated.y > bare.y,
                    "honor={honor}: the story viewport starts BELOW the plate. \
                     bare {bare:?} vs plated {plated:?}"
                );
                assert_eq!(
                    (plated.x, plated.right(), plated.bottom()),
                    (bare.x, bare.right(), bare.bottom()),
                    "honor={honor}: only the edge the plate stands on moves. \
                     bare {bare:?} vs plated {plated:?}"
                );
            }
        }
        assert!(
            plate_painted,
            "honor={honor} {side:?}: the story window's own plate is drawn by the ring. \
             Before SQ-0896 no band covered these cells and `blit_story_gfx` was reachable \
             from the RASTER path alone, so hybrid drew the prose over a picture it never drew."
        );
    }

    #[test]
    fn hybrid_ring_draws_art_the_game_painted_inside_the_story_window() {
        plate_in_story_window_case(true, PlateSide::Left);
        plate_in_story_window_case(true, PlateSide::Top);
    }

    #[test]
    fn hybrid_ring_draws_art_inside_the_story_window_with_game_colours_off() {
        plate_in_story_window_case(false, PlateSide::Left);
        plate_in_story_window_case(false, PlateSide::Top);
    }

    /// SQ-0515: a chrome grid window carrying `px_texts`, for the flood discriminator.
    fn flood_probe_window(w_px: u16, runs: Vec<crate::engine::PxText>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                win: 0,
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false, px_texts: runs,
            }),
        }
    }

    #[test]
    fn painted_screen_floods_only_full_width_reverse_rows() {
        use ratatui::style::Modifier;
        // Native 640px = 80 cells. A FULL-width window (w_px=640) with an all-reverse
        // row floods edge to edge; a NARROW window (w_px=169, ~26%) with a reverse row
        // stays a text-width block; a full-width row with a MIXED reverse/non-reverse
        // run set is NOT all-reverse, so it stays text-width too.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let native_w = 640u16;

        let full = flood_probe_window(640, vec![
            // Row 0: single reversed run → floods.
            crate::engine::PxText::derived(1, 1, "TITLE".into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT),
            // Row 1: one reversed + one non-reversed run → mixed, does NOT flood.
            crate::engine::PxText::derived(17, 1, "LEFT".into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT),
            crate::engine::PxText::derived(17, 401, "RIGHT".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
        ]);
        let narrow = flood_probe_window(169, vec![
            // Row 2: reversed run in a narrow window → text-width block, no flood.
            crate::engine::PxText::derived(33, 1, "SEL".into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT),
        ]);
        let chrome: Vec<&PositionedWindow> = vec![&full, &narrow];
        let runs: Vec<&crate::engine::PxText> = chrome
            .iter()
            .filter_map(|it| match &it.node {
                WinNode::Grid(g) => Some(g.px_texts.iter()),
                _ => None,
            })
            .flatten()
            .collect();

        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        draw_painted_screen(&runs, 0..u16::MAX, 0, area, &mut buf, base, TextInk::new(true, &colors), &chrome, native_w, zvm::screen::V6Cell::DEFAULT);

        let reversed_count = |y: u16| -> u16 {
            (0..area.width).filter(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED)).count() as u16
        };
        // Row 0: full-width all-reverse → flooded edge to edge.
        assert_eq!(reversed_count(0), area.width, "full-width all-reverse row floods every cell");
        // Row 1: full-width but MIXED reverse → only the "LEFT" glyphs reversed, no flood.
        assert!(reversed_count(1) > 0 && reversed_count(1) < area.width, "mixed-reverse row is not flooded: {} reversed", reversed_count(1));
        // Row 2: narrow window reverse → only "SEL" (3 cells) reversed, no flood.
        assert_eq!(reversed_count(2), 3, "narrow-window reverse row stays a text-width block");
    }

    /// A v6 Layered model whose chrome is fully TRANSPARENT, leaving the story
    /// window's box as a clear raster interior (the opaque `hybrid_v6_model` chrome
    /// insets `story_clear_native` to nothing). Story box native (43,39,234,160) →
    /// 29×20 raster cells (a 19-row body budget).
    fn raster_v6_model() -> ScreenModel {
        // Authentic 640×400 unit geometry (SQ-0479): the story window's 320px
        // height quantizes to 320/16 = 20 raster rows at the default cell.
        let chrome_img = image::RgbaImage::new(640, 400); // all alpha 0 (transparent)
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 80, h: 25, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        let story = PositionedWindow {
            x: 10, y: 4, w: 58, h: 20, x_px: 86, y_px: 78, w_px: 468, h_px: 320,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![chrome, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn raster_mode_publishes_scroll_geometry() {
        // SQ-0455: raster mode is still one rasterized pixel image (draw_v6_canvas),
        // but it now REPORTS the story box's scroll geometry so the shared scroll
        // keybindings and the [more] pager (SQ-0404) engage — replacing the old
        // behavior where the raster path returned None and published no geometry.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Raster;
        // 40 short lines overflow the 19-row body → real scroll capacity.
        for k in 0..40 {
            state.push_transcript(&format!("L{k}"));
        }

        let model = raster_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        let m = m.expect("raster path now reports story-box scroll metrics");
        assert_eq!(
            m.viewport_rows, 19,
            "story box is 20 raster rows (320px / the default cell's 16) minus the input line",
        );
        assert_eq!(m.total_rows, 40, "all 40 wrapped transcript rows counted");
        assert_eq!(m.max_scroll, 21, "40 total - 19 body");

        // Geometry is published (the raster grid is pixel-scaled, so area is the
        // whole pane — mouse mapping is approximate, scroll math is exact).
        let geom = state.transcript_geom.get().expect("raster mode publishes scroll geometry");
        assert_eq!(geom.total_rows, 40);
        assert_eq!(geom.first_abs_row, 21, "offset 0 → newest body at the bottom (40 - 19)");
    }

    #[test]
    fn v6_raster_gen_stable_when_idle_bumps_on_change() {
        // SQ-0469: the generation gate skips the whole rebuild+encode when nothing
        // changed, so an idle frame must produce an identical key while every real
        // input change must alter it.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        state.push_transcript("You are in a maze.");
        let area = Rect::new(0, 0, 40, 25);

        let model = raster_v6_model();
        let items = match model.root {
            WinNode::Layered(v) => v,
            other => panic!("expected Layered, got {other:?}"),
        };

        let base = v6_raster_gen(&items, &state, area, &picker);
        // Idle: recomputing with no change is identical → the gate skips the frame.
        assert_eq!(base, v6_raster_gen(&items, &state, area, &picker), "idle frame → same key");

        // A v6 run change (here a picture repaint bumps its version stamp).
        let mut mutated = items.clone();
        if let WinNode::Graphics(g) = &mut mutated[0].node {
            g.version = g.version.wrapping_add(1);
        }
        assert_ne!(base, v6_raster_gen(&mutated, &state, area, &picker), "a v6 window change bumps the key");

        // A transcript append.
        let mut s2 = AppState::default();
        s2.colors = crate::colors::ColorScheme::terminal_default();
        s2.push_transcript("You are in a maze.");
        s2.push_transcript("A grue lurks nearby.");
        assert_ne!(base, v6_raster_gen(&items, &s2, area, &picker), "new transcript output bumps the key");

        // A keystroke on the live input line.
        state.input.value.push('x');
        assert_ne!(base, v6_raster_gen(&items, &state, area, &picker), "an input-line keystroke bumps the key");
        state.input.value.clear();

        // A pane resize.
        assert_ne!(base, v6_raster_gen(&items, &state, Rect::new(0, 0, 41, 25), &picker), "a resize bumps the key");

        // Scrolling the transcript back.
        state.transcript_scroll = 3;
        assert_ne!(base, v6_raster_gen(&items, &state, area, &picker), "a scroll change bumps the key");
        state.transcript_scroll = 0;

        // Arming the word reveal (SQ-1138). This is the one input that moves NOTHING
        // else in this key — no window, no run, no pixel of the model, no transcript
        // line and no input character — so without its own clause the gate answers
        // "nothing changed", the composite is never rebuilt, and the reveal is dark
        // on the raster surface while every other reason for it is correct. That is
        // the second half of the defect, and it is invisible to a test of the DRAW.
        assert!(state.reveal.is_none(), "the base key was taken with nothing lit");
        // `RevealTier` was removed by SQ-1135, which landed alongside this in the
        // same wave: with the reveal annotating by VOCABULARY there is one tier
        // left, so the field went with it. Both lanes compiled alone and the merge
        // was textually clean — this line is the whole of the semantic conflict.
        state.reveal = Some(crate::reveal::Reveal {
            words: ["lantern".to_string()].into_iter().collect(),
            until: std::time::Instant::now() + crate::reveal::REVEAL_HOLD,
        });
        let armed = v6_raster_gen(&items, &state, area, &picker);
        assert_ne!(base, armed, "arming a reveal bumps the key");
        // …and a DIFFERENT set of lit words is a different frame again: the words are
        // hashed by content, so a reveal re-armed on a new screenful repaints rather
        // than reusing the last one's highlights.
        state.reveal.as_mut().expect("lit").words = ["sceptre".to_string()].into_iter().collect();
        assert_ne!(armed, v6_raster_gen(&items, &state, area, &picker), "different words, different key");
        // Going out returns to exactly the unlit key, so the prose repaints in the
        // story's own colour when the reveal expires.
        state.reveal = None;
        assert_eq!(base, v6_raster_gen(&items, &state, area, &picker), "a reveal that went out restores the idle key");
    }

    /// A synthetic v6 `Layered` model for the cell-path tests: a chrome
    /// `Grid` carrying one status px-run at native (1,1) → cell (0,0), plus a
    /// primary story `Buffer` one native row below it. No decorative graphics
    /// window. Pixel geometry is the authentic 8×16 v6 text cell (SQ-0479), so
    /// the status window really does occupy the row ABOVE the story — the
    /// relation the cell-path band split reads (SQ-0549).
    fn cell_path_v6_model() -> ScreenModel {
        let status = PositionedWindow {
            x: 0, y: 0, w: 40, h: 1, x_px: 0, y_px: 0, w_px: 320, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                win: 0,
                fill: None,
                cols: 40, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText::derived(1, 1, "SCORE 10".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT),
                ],
            }),
        };
        let story = PositionedWindow {
            x: 0, y: 1, w: 40, h: 24, x_px: 0, y_px: 16, w_px: 320, h_px: 384,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![status, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn cell_path_renders_full_pane_transcript_with_status_band_and_no_graphics() {
        // Retargeted from `frameless_renders_…` (SQ-0895). The property is the
        // CELL PATH's layout — chrome text collapsed to a compact status band,
        // story as a normal full-pane terminal transcript, no pixel chrome at
        // all — which SQ-0461's removed mode was only one of four ways to reach.
        // Driven here through the MODAL OVERLAY route so the "a picker is present
        // and it still bypasses both pixel paths" half of the original assertion
        // survives: image placements draw above terminal cells, so a dialog over
        // the story pane needs the cells (`screen.rs:670-673`).
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.overlays.reset_dialog = true;
        assert!(state.any_modal_overlay_open(), "the overlay route into the cell path is open");
        state.push_transcript("HELLO STORY WORLD");

        let model = cell_path_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );
        let m = m.expect("the cell path returns the primary-buffer transcript metrics");

        // The transcript occupies the FULL pane below the one-row status band —
        // NOT an inset chrome-ring viewport (hybrid) and NOT a pixel raster. The
        // transcript always reserves the rightmost column as a scrollbar gutter,
        // so a full-pane body is `area.width - 1` wide (vs hybrid's much-narrower
        // inset viewport).
        let geom = state.transcript_geom.get().expect("the cell path publishes transcript geometry");
        let vp = geom.area;
        assert_eq!(vp.x, area.x, "transcript is flush to the left pane edge (not inset)");
        assert_eq!(vp.width, area.width - 1, "transcript spans the full pane width minus the scrollbar gutter");
        assert_eq!(vp.y, area.y + 1, "transcript starts below the 1-row status band");
        assert_eq!(m.viewport_rows, area.height - 1, "metrics report the full-pane body height below the band");

        // The whole pane rendered as real terminal cells: the status run sits in
        // the top row and the story text renders as selectable text below it.
        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect::<String>() + "\n")
            .collect();
        assert!(screen.contains("SCORE 10"), "status band renders as terminal text, screen:\n{screen}");
        assert!(screen.contains("HELLO STORY WORLD"), "story renders as a full-pane transcript, screen:\n{screen}");
    }

    #[test]
    fn cell_path_publishes_a_click_map_covering_the_pane() {
        // SQ-0532/A-F4: the cell path draws no game image, so it used to record
        // NO click map at all — v6 mouse input was dead there while raster and
        // hybrid both worked. It now records the proportional pane→native map,
        // and a click maps into the game-pixel rect the clicked cell stands for.
        //
        // Retargeted from `frameless_publishes_…` (SQ-0895) onto the NO-PICKER
        // route, which the original's own comment already named as the other way
        // in. Between this and `cell_path_renders_full_pane_transcript_…` (the
        // overlay route) both surviving entrances stay covered.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = None; // no image protocol
        state.push_transcript("HELLO STORY WORLD");

        let model = cell_path_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let mut win_rects = Vec::new();
        let _ = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &mut win_rects, &state.colors,
        );

        let map = state
            .graphics_render
            .borrow()
            .last_v6_map
            .clone()
            .expect("the cell path publishes a v6 click map");
        // The model's native extent is its 320x200 game-pixel screen.
        let (nw, nh) = crate::render::v6_layout::native_extent(match &model.root {
            WinNode::Layered(items) => items,
            _ => unreachable!("cell_path_v6_model is Layered"),
        }, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert_eq!((map.canvas, map.screen), ((nw, nh), (nw, nh)));

        // Top-left cell → the top-left game pixel (1-based origin, ZMSD §8.8.1).
        let (gx, gy) = map.map_click(area.x, area.y).expect("a click inside the pane maps");
        assert!(gx <= nw / area.width + 1 && gy <= nh / area.height + 1,
            "top-left cell maps into the top-left game-pixel cell, got ({gx},{gy})");
        // A known interior cell maps into the game-pixel rect it stands for: cell
        // (col, row) covers native x in [col/W, (col+1)/W) of the screen width.
        let (col, row) = (area.x + 30, area.y + 20);
        let (gx, gy) = map.map_click(col, row).expect("interior click maps");
        let (lo_x, hi_x) = (nw as u32 * 30 / 40, nw as u32 * 31 / 40);
        let (lo_y, hi_y) = (nh as u32 * 20 / 25, nh as u32 * 21 / 25);
        assert!((lo_x..=hi_x).contains(&(gx as u32 - 1)), "x {gx} in {lo_x}..={hi_x}");
        assert!((lo_y..=hi_y).contains(&(gy as u32 - 1)), "y {gy} in {lo_y}..={hi_y}");
        // Outside the pane is still a miss (the app falls back to selection).
        assert_eq!(map.map_click(area.right() + 2, area.y), None);
    }

    // `frameless_no_images_equals_cell_fallback` was DELETED here by SQ-0895
    // rather than retargeted, because it no longer describes anything. Its whole
    // content was `render(Hybrid) == render(Frameless)` with no picker — an
    // assertion that the mode COLLAPSED onto the fallback when there were no
    // images to differentiate them. With the mode gone the two sides are the same
    // expression and the comparison is vacuous. The surviving half of what it
    // covered — that a no-picker hybrid config really does render the cell
    // fallback — is asserted directly by `cell_path_publishes_a_click_map_…`,
    // which drives exactly that configuration.

    // ── Anchored status band (SQ-0467) ──────────────────────────────────────────

    /// One px-run at native pixel `(x, y)` (1-based) carrying `text`.
    fn run(x: u16, y: u16, text: &str) -> crate::engine::PxText {
        crate::engine::PxText::derived(y, x, text.into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT)
    }

    /// Render `runs` as an anchored band over a `w`-cell pane at native width
    /// `ncols` cells, returning the top row's text (trailing spaces trimmed off
    /// the right only via the caller) plus the raw buffer for column probing.
    fn band_row(runs: &[crate::engine::PxText], ncols: u32, w: u16) -> (String, u16) {
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, w, 6);
        let mut buf = Buffer::empty(area);
        let style = ratatui::style::Style::default();
        let colors = crate::colors::ColorScheme::terminal_default();
        let rows_used = draw_anchored_status_band(&refs, ncols, 4, area, &mut buf, style, TextInk::new(true, &colors));
        let text: String = (0..w).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect();
        (text, rows_used)
    }

    #[test]
    fn anchored_band_shogun_shape_left_center_right() {
        // Native 40-cell screen: a location/name run at the far left, a centered
        // title, and two right-side status runs. Left flush col 0, title centered,
        // the two right runs two-space joined and ending flush at the last column.
        let runs = vec![
            run(1, 1, "Shogun"),           // start col 0 → LEFT
            run(129, 1, "The Tale"),       // start col 16, end col 24 → CENTER
            run(233, 1, "Score: 0"),       // start col 29, end col 37 → RIGHT
            run(281, 1, "Moves: 1"),       // start col 35, end col 43 → RIGHT
        ];
        let (row, rows_used) = band_row(&runs, 40, 80);
        assert_eq!(rows_used, 1);
        assert!(row.starts_with("Shogun"), "left run flush at col 0: {row:?}");
        // Right group: two runs joined by exactly two spaces, ending flush right.
        assert!(row.trim_end().ends_with("Score: 0  Moves: 1"), "right group joined + flush: {row:?}");
        assert_eq!(row.chars().count(), 80);
        assert_eq!(&row[row.len() - "Moves: 1".len()..], "Moves: 1", "right group ends at the last column");
        // Title centered within ±1 of the pane centre.
        let title_start = row.find("The Tale").expect("centered title present");
        let expected = (80 - "The Tale".chars().count()) / 2;
        assert!((title_start as i32 - expected as i32).abs() <= 1, "title centered (at {title_start}, want ~{expected})");
    }

    #[test]
    fn anchored_band_zork0_shape_location_and_right_status() {
        // Location at the left, score/moves at the right — no centre group.
        let runs = vec![
            run(9, 1, "West of House"),   // start col 1 → LEFT
            run(241, 1, "Score: 0"),      // → RIGHT
            run(297, 1, "Moves: 3"),      // → RIGHT
        ];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with("West of House"), "location flush left: {row:?}");
        assert!(row.trim_end().ends_with("Score: 0  Moves: 3"), "score/moves joined + flush right: {row:?}");
        assert_eq!(&row[row.len() - "Moves: 3".len()..], "Moves: 3");
    }

    #[test]
    fn anchored_band_narrow_pane_priority_and_truncation() {
        // A 28-col pane: LEFT stays intact, RIGHT truncates from its left edge to
        // keep a space from LEFT, CENTER drops because it can't fit between them.
        let runs = vec![
            run(1, 1, "A Fairly Long Location"), // 22 chars → LEFT (col 0)
            run(129, 1, "Title"),                // CENTER (dropped, no room)
            run(281, 1, "Moves: 100"),           // RIGHT (10 chars, must truncate)
        ];
        let (row, _) = band_row(&runs, 40, 28);
        assert_eq!(row.chars().count(), 28);
        assert!(row.starts_with("A Fairly Long Location"), "LEFT intact: {row:?}");
        assert!(!row.contains("Title"), "CENTER dropped when it can't fit: {row:?}");
        // RIGHT truncated from the left, still flush to the last column, and never
        // overwriting LEFT: a space separates them.
        assert_eq!(row.chars().nth(22), Some(' '), "≥1 space between LEFT and RIGHT");
        assert!(row.ends_with(|c: char| c != ' '), "RIGHT still flush at the last column: {row:?}");
        // Only the last 5 cols hold RIGHT's tail (28 - 22 - 1 = 5 chars survive).
        let tail: String = row.chars().skip(23).collect();
        assert_eq!(tail, ": 100", "RIGHT truncated from its left edge to the fitting tail");
    }

    /// SQ-0717: a line the game centred on its own screen stays centred, however
    /// far left it begins. Shogun's frozen title header (SQ-0697) is the case —
    /// nine cursor-centred lines that the thirds rule sorted by their START, so the
    /// long ones began left of the left-third boundary and flushed to col 0 while
    /// the shortest ended past the right two-thirds and flushed right.
    #[test]
    fn anchored_band_keeps_a_line_the_game_centred() {
        // Shogun's own columns, on its 640px (80-cell) screen.
        let lines: [(u16, &str); 3] = [
            (297, "SHOGUN"),                                             // col 37, well inside
            (105, "Original Literary Work Copyright 1975 by James Clavell"), // col 13 → was LEFT
            (209, "IBM Interpreter version 6.65"),                       // ends col 54 → was RIGHT
        ];
        for w in [80u16, 120] {
            for (x, text) in lines {
                let (row, _) = band_row(&[run(x, 1, text)], 80, w);
                let at = row.find(text).unwrap_or_else(|| panic!("{text:?} painted at {w} cols: {row:?}"));
                let want = (w as usize - text.chars().count()) / 2;
                assert!(
                    (at as i32 - want as i32).abs() <= 1,
                    "{text:?} stays centred at {w} cols (at {at}, want ~{want}): {row:?}"
                );
            }
        }
    }

    /// …and the centring exemption does not loosen a real bar. A field that begins
    /// at the screen's left edge is LEFT even when its right margin happens to
    /// match, and edge-anchored status fields keep their thirds classification.
    #[test]
    fn anchored_band_centring_exemption_spares_edge_anchored_fields() {
        // A rule drawn from col 0: margins 0 and 4 — not centred, still LEFT.
        let bar = "=".repeat(36);
        let (row, _) = band_row(&[run(1, 1, &bar)], 40, 80);
        assert!(row.starts_with(&bar), "an edge-anchored rule is not 'centred': {row:?}");
        // Location left, score/moves right — the classic bar, unmoved.
        let runs = vec![run(9, 1, "West of House"), run(241, 1, "Score: 0"), run(297, 1, "Moves: 3")];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with("West of House"), "location still flush left: {row:?}");
        assert!(row.trim_end().ends_with("Score: 0  Moves: 3"), "score/moves still flush right: {row:?}");
    }

    /// SQ-0712: the band is measured before the story area is sized and painted
    /// after the erase fills, so `anchored_band_rows` has to agree with what
    /// `draw_anchored_status_band` actually uses — a drift between them mis-sizes
    /// the transcript or strands the bar off-pane.
    #[test]
    fn anchored_band_measurement_matches_the_draw() {
        let cases: Vec<(Vec<crate::engine::PxText>, u16, u16)> = vec![
            // One bar row at the top of a 4-row band.
            (vec![run(1, 1, "Loc"), run(281, 1, "Moves: 1")], 4, 6),
            // Two rows, one apart.
            (vec![run(1, 1, "Row0"), run(1, 17, "Row1")], 4, 6),
            // A gap row in the middle still counts toward the span.
            (vec![run(1, 1, "Row0"), run(1, 49, "Row3")], 4, 6),
            // Arthur's shape: nothing until native row 12, band 13 deep.
            (vec![run(33, 193, "Churchyard")], 13, 6),
            // Blank-only runs paint nothing and measure nothing.
            (vec![run(129, 1, "   ")], 4, 6),
            // Nothing at all.
            (vec![], 4, 6),
            // The span is clamped to a pane shorter than the band.
            (vec![run(1, 1, "Row0"), run(1, 81, "Row5")], 8, 3),
        ];
        for (runs, band_rows, pane_h) in cases {
            let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
            let area = Rect::new(0, 0, 80, pane_h);
            let mut buf = Buffer::empty(area);
            let colors = crate::colors::ColorScheme::terminal_default();
            let drawn = draw_anchored_status_band(
                &refs, 40, band_rows, area, &mut buf, ratatui::style::Style::default(), TextInk::new(true, &colors),
            );
            assert_eq!(
                anchored_band_rows(&refs, band_rows, pane_h),
                drawn,
                "measured rows must equal drawn rows for {:?} (band {band_rows}, pane {pane_h})",
                runs.iter().map(|t| (t.y, t.x, &t.text)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn anchored_band_multi_row() {
        // Runs on native rows 0 and 1 (y=1 and y=17, one 16px cell apart) each
        // render on their own band row, and rows_used reports 2.
        let runs = [run(1, 1, "Row0Left"), run(233, 1, "Score: 0"), run(1, 17, "Row1Left")];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        let colors = crate::colors::ColorScheme::terminal_default();
        let rows_used = draw_anchored_status_band(&refs, 40, 4, area, &mut buf, ratatui::style::Style::default(), TextInk::new(true, &colors));
        assert_eq!(rows_used, 2, "two native rows populated");
        let r0: String = (0..80).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect();
        let r1: String = (0..80).map(|x| buf.cell((x, 1)).unwrap().symbol().to_string()).collect();
        assert!(r0.starts_with("Row0Left"), "row 0 left: {r0:?}");
        assert!(r0.trim_end().ends_with("Score: 0"), "row 0 right flush: {r0:?}");
        assert!(r1.starts_with("Row1Left"), "row 1 left: {r1:?}");
    }

    #[test]
    fn anchored_band_wide_run_counts_left_not_stretched() {
        // A full-width bar (spans most of the row) anchors LEFT at col 0 rather
        // than being treated as a centred title.
        let bar = "=".repeat(36);
        let runs = vec![run(1, 1, &bar)];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with(&bar), "wide bar flush at col 0, not centred: {row:?}");
    }

    #[test]
    fn anchored_band_skips_blank_runs() {
        // A whitespace-only run must not drag a group around or count as painted.
        let runs = vec![run(129, 1, "   ")];
        let (row, rows_used) = band_row(&runs, 40, 80);
        assert_eq!(rows_used, 0, "blank-only band paints nothing");
        assert!(row.trim().is_empty(), "no text painted: {row:?}");
    }

    #[test]
    fn anchored_band_tiny_pane_no_panic() {
        // Width 1 and 2 must not panic and LEFT still wins.
        for w in [1u16, 2] {
            let runs = vec![run(1, 1, "Loc"), run(281, 1, "Moves: 1")];
            let (_row, _) = band_row(&runs, 40, w);
        }
    }

    /// A reverse-video px-run at native pixel `(x, y)` (1-based) carrying `text`.
    fn rev_run(x: u16, y: u16, text: &str) -> crate::engine::PxText {
        crate::engine::PxText::derived(y, x, text.into(), 1, 0, 0, zvm::screen::V6Cell::DEFAULT)
    }

    #[test]
    fn painted_screen_fills_reverse_video_gaps_between_words() {
        use ratatui::style::Modifier;
        // SQ-0484 defect 2: a highlighted (reverse-video) menu item paints each
        // word AND each inter-word space as a SEPARATE run. Dropping the blank
        // runs left the selection bar reversed behind the glyphs but not the gaps
        // ("moth-eaten"). The reversed blank runs must now be stamped, so the whole
        // bar reads as one solid reverse block. A NON-reverse blank stays a no-op.
        //
        // Row 1 (native y=17 → cell row 1): "GO" at cols 0..2, a reverse space at
        // col 2, "IN" at cols 3..5 — the gap cell (2) must carry REVERSED.
        let runs = [
            rev_run(1, 17, "GO"),
            rev_run(17, 17, " "),
            rev_run(25, 17, "IN"),
        ];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let colors = crate::colors::ColorScheme::terminal_default();
        draw_painted_screen(&refs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), TextInk::new(true, &colors), &[], 0, zvm::screen::V6Cell::DEFAULT);
        // Every cell of the bar (cols 0..5) is REVERSED — including the gap at col 2.
        for x in 0..5u16 {
            assert!(
                buf.cell((x, 1)).unwrap().modifier.contains(Modifier::REVERSED),
                "col {x} of the reverse selection bar is reversed (gap included)"
            );
        }
        // SQ-0490: when the selection moves away the game repaints the row's gaps
        // as PLAIN spaces. Those must stamp too (painter semantics) — repainting
        // over the earlier reversed cells — or the old bar's gap cells stay
        // reversed forever. Same runs re-painted plain, in the same buffer:
        let plain: Vec<crate::engine::PxText> = runs
            .iter()
            .map(|t| crate::engine::PxText { style: 0, ..t.clone() })
            .collect();
        let prefs: Vec<&crate::engine::PxText> = plain.iter().collect();
        draw_painted_screen(&prefs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), TextInk::new(true, &colors), &[], 0, zvm::screen::V6Cell::DEFAULT);
        assert!(
            !buf.cell((2, 1)).unwrap().modifier.contains(Modifier::REVERSED),
            "the gap cell is repainted plain once the selection moves away (SQ-0490)"
        );
    }

    // ── v6 run → cell Style resolution (SQ-0488) ────────────────────────────────

    #[test]
    fn v6_run_style_explicit_standard_sets_channel() {
        // A run with an explicit Standard-palette fg resolves that channel to the
        // palette colour (Zork0's compass letters carry Standard colours), leaving
        // the themed bg intact. Standard(3) is a real palette choice.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let fg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3));
        let s = v6_run_style(base, fg, 0, 0, TextInk::new(true, &colors));
        assert_eq!(s.fg, Some(crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(3), &colors)));
        assert_eq!(s.bg, base.bg, "unset bg keeps the theme background");
    }

    #[test]
    fn v6_run_style_true_colour_sets_rgb() {
        // A True24 (24-bit) run resolves to the exact RGB.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x40_2010));
        let s = v6_run_style(base, 0, bg, 0, TextInk::new(true, &colors));
        assert_eq!(s.bg, Some(ratatui::style::Color::Rgb(0x40, 0x20, 0x10)));
        assert_eq!(s.fg, base.fg, "unset fg keeps the theme foreground");
    }

    #[test]
    fn v6_run_style_unset_and_default_sentinels_keep_theme() {
        // Default (0) and Standard 0/1 ("current"/"default") are inheritance, not
        // choices — every channel keeps the theme base. Shogun sets no colours, so
        // its Default/Default runs must land here.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        for packed in [
            0u32,
            crate::state::pack_zcolour(zvm::screen::ZColour::Standard(0)),
            crate::state::pack_zcolour(zvm::screen::ZColour::Standard(1)),
        ] {
            let s = v6_run_style(base, packed, packed, 0, TextInk::new(true, &colors));
            assert_eq!(s.fg, base.fg, "sentinel {packed:#x} keeps theme fg");
            assert_eq!(s.bg, base.bg, "sentinel {packed:#x} keeps theme bg");
            assert!(!s.add_modifier.contains(ratatui::style::Modifier::REVERSED));
        }
    }

    #[test]
    fn v6_run_style_reverse_bit_toggles_modifier() {
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let rev = v6_run_style(base, 0, 0, 1, TextInk::new(true, &colors));
        assert!(rev.add_modifier.contains(ratatui::style::Modifier::REVERSED), "reverse bit adds REVERSED");
        let plain = v6_run_style(base.add_modifier(ratatui::style::Modifier::REVERSED), 0, 0, 0, TextInk::new(true, &colors));
        assert!(plain.sub_modifier.contains(ratatui::style::Modifier::REVERSED), "no reverse bit removes REVERSED");
    }

    #[test]
    fn v6_run_style_carries_bold_and_italic() {
        // ZMSD §8.7.1 styles: bit 2 = Bold, bit 4 = Italic (bit 1 = Reverse
        // Video, bit 8 = Fixed Pitch). The v6 cell paths used to drop bold and
        // italic entirely, rendering emphasised menu text as roman.
        use ratatui::style::Modifier;
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        assert!(v6_run_style(base, 0, 0, 2, TextInk::new(true, &colors)).add_modifier.contains(Modifier::BOLD));
        assert!(v6_run_style(base, 0, 0, 4, TextInk::new(true, &colors)).add_modifier.contains(Modifier::ITALIC));
        // Combined with reverse video, and unaffected by the colour gate.
        let all = v6_run_style(base, 0, 0, 1 | 2 | 4, TextInk::new(false, &colors)).add_modifier;
        assert!(all.contains(Modifier::BOLD) && all.contains(Modifier::ITALIC) && all.contains(Modifier::REVERSED));
        // Fixed-pitch (8) alone still adds nothing in a monospaced terminal.
        let fixed = v6_run_style(base, 0, 0, 8, TextInk::new(true, &colors));
        assert!(!fixed.add_modifier.contains(Modifier::BOLD) && !fixed.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn v6_run_style_asks_the_terminal_for_italics_and_never_both() {
        // SQ-1028: §8.7.1 lets an interpreter render the Italic bit broadly, and the
        // rule here is a real italic FACE where one is available, an underline where
        // none is, and never a slope we synthesised. On a cell path the face is the
        // player's terminal font, so the bit is SGR 3 and nothing else — in
        // particular the two renderings are alternatives, never both, since doing
        // both is neither.
        use ratatui::style::Modifier;
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let emph = v6_run_style(base, 0, 0, 4, TextInk::new(true, &colors)).add_modifier;
        assert!(emph.contains(Modifier::ITALIC), "the terminal's own italic face is what §8.7.1's bit asks for here");
        assert!(!emph.contains(Modifier::UNDERLINED), "…and it is not also underlined — one bit, one rendering");
    }

    #[test]
    fn v6_run_style_colours_off_returns_theme_base() {
        // honor=false ⇒ explicit colours are ignored, matching every other engine's
        // honor_game_colours gate (Glulx cell_style, the v1-5 grid).
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let fg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3));
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x123456));
        let s = v6_run_style(base, fg, bg, 0, TextInk::new(false, &colors));
        assert_eq!(s.fg, base.fg);
        assert_eq!(s.bg, base.bg);
    }

    #[test]
    fn v6_painted_screen_explicit_run_paints_game_colour() {
        // End-to-end through draw_painted_screen: an explicit Standard-3 fg run
        // stamps with the palette colour, while a Default/Default run keeps the
        // theme base (Shogun's regression pin — its runs are all Default/Default).
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let coloured = crate::engine::PxText::derived(1, 1, "N".into(), 0, crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3)), 0, zvm::screen::V6Cell::DEFAULT);
        let plain = crate::engine::PxText::derived(1, 25, "X".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT);
        let refs: Vec<&crate::engine::PxText> = vec![&coloured, &plain];
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        draw_painted_screen(&refs, 0..u16::MAX, 0, area, &mut buf, base, TextInk::new(true, &colors), &[], 0, zvm::screen::V6Cell::DEFAULT);
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(3), &colors),
            "explicit game colour reaches the buffer cell"
        );
        // The plain run at col 3 (x=25 → (25-1)/8 = 3) keeps the theme fg.
        assert_eq!(buf.cell((3, 0)).unwrap().fg, base.fg.unwrap_or(ratatui::style::Color::Reset), "Default run stays theme-styled");
    }

    #[test]
    fn v6_anchored_band_honours_explicit_run_colour() {
        // The frameless status band resolves an explicit run's colour for its row
        // (Zork0's ribbon labels), while Shogun's Default/Default band keeps theme.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let coloured = crate::engine::PxText::derived(1, 1, "West of House".into(), 0, crate::state::pack_zcolour(zvm::screen::ZColour::Standard(4)), 0, zvm::screen::V6Cell::DEFAULT);
        let refs: Vec<&crate::engine::PxText> = vec![&coloured];
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        draw_anchored_status_band(&refs, 40, 4, area, &mut buf, base, TextInk::new(true, &colors));
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(4), &colors),
            "band row adopts the explicit run colour"
        );
        // Shogun regression: a Default/Default run yields exactly the theme fg.
        let plain = crate::engine::PxText::derived(1, 1, "Shogun".into(), 0, 0, 0, zvm::screen::V6Cell::DEFAULT);
        let prefs: Vec<&crate::engine::PxText> = vec![&plain];
        let mut buf2 = Buffer::empty(area);
        draw_anchored_status_band(&prefs, 40, 4, area, &mut buf2, base, TextInk::new(true, &colors));
        assert_eq!(buf2.cell((0, 0)).unwrap().fg, base.fg.unwrap_or(ratatui::style::Color::Reset), "Default band stays theme-styled");
    }

    /// TEMP measurement harness (SQ-0469). Times the three raster phases —
    /// canvas BUILD (chrome + wrap + glyph blit), content HASH, and
    /// RESIZE+ENCODE — for a real v6 story at a large pane. Run with:
    ///   cargo test -p app bench_v6_raster_phases -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_v6_raster_phases() {
        use crate::engine::Engine;
        use std::hash::{Hash, Hasher};
        use std::time::Instant;
        let fg = image::Rgba([220u8, 220, 220, 255]);
        let bg = image::Rgba([0u8, 0, 0, 255]);
        for path in ["stories/zork0-r393-s890714.z6", "stories/shogun-r322-s890706.z6"] {
            let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(path);
            let Ok(bytes) = std::fs::read(&full) else {
                println!("SKIP {path}: not found");
                continue;
            };
            let mut sess = match crate::session::GameSession::new(bytes, true, false, None) {
                Ok(s) => s,
                Err(e) => {
                    println!("SKIP {path}: {e:?}");
                    continue;
                }
            };
            for cmd in ["look", "open mailbox", "look"] {
                let _ = sess.submit(cmd);
            }
            let model = sess.screen();
            let items = match &model.root {
                WinNode::Layered(v) => v.clone(),
                other => {
                    println!("SKIP {path}: root is {other:?}, not Layered");
                    continue;
                }
            };
            let native = crate::render::v6_layout::native_extent(&items, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));

            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            for i in 0..2000 {
                state
                    .transcript
                    .push(format!("The quick brown fox line {i} jumps over the lazy dog by the white house door."));
            }
            state.transcript_styles.resize(state.transcript.len(), None);
            state.transcript_runs.resize(state.transcript.len(), Vec::new());
            state.transcript_para.resize(state.transcript.len(), crate::state::ParaFmt::default());
            state.transcript_images.resize(state.transcript.len(), None);

            // A large pane: 220x64 cells at halfblocks 10x20 px = 2200x1280 device.
            let picker = state.game_picker.clone().unwrap();
            let area = Rect::new(0, 0, 220, 64);

            // Build closure: replicate the raster branch's canvas construction.
            let build = || {
                let layout = crate::render::v6_layout::classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
                let mut canvas = crate::render::v6_layout::build_chrome_canvas(&layout.chrome, native, fg, bg, &state.colors, crate::render::v6_layout::TextLayer::All, &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
                if let Some((sx, sy, sw, sh)) = crate::render::v6_layout::story_clear_native(layout.story, &canvas) {
                    let cols = (sw / 8).max(1) as u16;
                    let rows = (sh / 8).max(1) as u16;
                    let (main, _) = build_main_text(&state, cols, rows);
                    crate::render::v6_layout::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, fg, &[], &crate::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT), None);
                }
                canvas
            };

            const N: u32 = 30;
            // Phase GEN (SQ-0469): the whole cost of an idle/unchanged frame after
            // the gate — no build, no hash, no encode.
            let t = Instant::now();
            let mut gsum = 0u64;
            for _ in 0..N {
                gsum ^= v6_raster_gen(&items, &state, area, &picker);
            }
            let gen_us = t.elapsed().as_micros() as f64 / N as f64;
            std::hint::black_box(gsum);

            // Phase BUILD.
            let t = Instant::now();
            let mut canvas = build();
            for _ in 1..N {
                canvas = build();
            }
            let build_us = t.elapsed().as_micros() as f64 / N as f64;

            // Phase HASH.
            let t = Instant::now();
            let mut hsum = 0u64;
            for _ in 0..N {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                canvas.as_raw().hash(&mut h);
                hsum ^= h.finish();
            }
            let hash_us = t.elapsed().as_micros() as f64 / N as f64;
            std::hint::black_box(hsum);

            // Phase RESIZE+ENCODE (uncapped, as shipped).
            let fs = picker.font_size();
            let box_w = area.width as u32 * fs.width.max(1) as u32;
            let box_h = area.height as u32 * fs.height.max(1) as u32;
            let (cw, ch) = (canvas.width(), canvas.height());
            let encode = |cap: f64| {
                let scale = ((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64)).max(1.0).min(cap);
                let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
                let scaled = image::imageops::resize(&canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
                let img = image::DynamicImage::ImageRgba8(scaled);
                let _ = picker.new_protocol(img, ratatui::layout::Size::new(area.width, area.height), ratatui_image::Resize::Fit(None));
            };
            let t = Instant::now();
            for _ in 0..N {
                encode(f64::INFINITY);
            }
            let enc_us = t.elapsed().as_micros() as f64 / N as f64;
            let t = Instant::now();
            for _ in 0..N {
                encode(4.0);
            }
            let enc4_us = t.elapsed().as_micros() as f64 / N as f64;

            println!(
                "\n=== {path} ===\n native canvas: {}x{}  pane device: {}x{}\n GEN (idle key):   {gen_us:>9.1} us/frame\n BUILD:            {build_us:>9.1} us/frame\n HASH:             {hash_us:>9.1} us/frame\n ENCODE (uncap):   {enc_us:>9.1} us/frame\n ENCODE (cap 4x):  {enc4_us:>9.1} us/frame\n --- BEFORE (no gate; build+hash every frame) ---\n IDLE / keystroke frame  = {:.1} us (build+hash on main)\n CHANGED frame           = {:.1} us (build+hash+encode on main)\n --- AFTER (SQ-0469 gate + cap + worker) ---\n IDLE frame              = {gen_us:.1} us (gen key only)\n KEYSTROKE/CHANGED frame = {:.1} us on main (gen+build; capped encode {enc4_us:.1} us OFF-thread)",
                native.0, native.1, box_w, box_h,
                build_us + hash_us,
                build_us + hash_us + enc_us,
                gen_us + build_us,
            );
        }
    }
}
