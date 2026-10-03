//! Telling the story how big its pane is (SQ-1539).
//!
//! The host decides the pane — the TUI measures its story pane every frame, a
//! GUI or a browser measures its own — and says so in character cells. What the
//! story is told from that is a rule, and lives here: a v4+ Z-machine story gets
//! its header's `$20`/`$21` (ZMSD §8.4, floored at the width it booted with,
//! SQ-0679), and a Glulx story is resized and re-arranged (SQ-0201).
//!
//! The TUI debounces the Glulx half (a drag would otherwise run the game's
//! redraw on every tick) and calls [`resize_glulx`] once the size settles; the
//! Z-machine half costs two header bytes and follows every frame. A host with
//! no drag to debounce calls [`set_story_pane`] for both.

use ratatui::layout::Rect;

use crate::engine::Engine;
use crate::engine_helpers::{glulx_session_opt, zvm_session_opt};
use crate::glulx_session::GlulxSession;
use crate::state::AppState;

/// Report the story pane's size, `(cols, rows)` in cells, to a v4/v5/v7/v8
/// Z-machine story (ZMSD §8.4 — SQ-0532/A-F1). Returns `true` when the header
/// changed (the upper window's width follows it).
///
/// §8.4: the interpreter "may change the exact dimensions whenever it likes but
/// must write the current height (in lines) and width (in characters) into bytes
/// $20 and $21 in the header."
///
/// There is no settle timer: this writes two header bytes rather than running
/// the game's re-layout code, so it is free to track every intermediate size
/// during a drag. It also carries no cached "last applied" size — it compares
/// against what the header currently SAYS, so a fresh boot after `@restart`
/// (whose new `Machine` re-seeds the fallback) is corrected on the next call
/// without the restart path having to know about any of this.
///
/// Skipped for v6, whose fixed 640×400 pixel screen is scaled into the pane
/// rather than measured from it, and for v1–3, which have no such header fields.
///
/// What is declared is [`declared_story_screen_dims`], not the raw pane
/// measurement: the width is floored at the columns the story booted with
/// (SQ-0679) — whatever `GameSession::boot_screen_cols` says THIS session
/// actually booted at, 80 by default or a narrower/wider pre-boot-seeded pane
/// (SQ-0680) — so narrowing the pane can never move a v4/v5 status routine's
/// baked-in field columns outside the window.
///
/// [`declared_story_screen_dims`]: crate::render::screen::declared_story_screen_dims
pub fn sync_zvm_screen_dims(session: &mut dyn Engine, state: &AppState, (cols, rows): (u16, u16)) -> bool {
    let Some(gs) = session.as_any().downcast_ref::<crate::session::GameSession>() else {
        return false;
    };
    let version = gs.machine.mem.version();
    if version < 4 || version == 6 {
        return false;
    }
    let Some((rows, cols)) = crate::render::screen::declared_story_screen_dims(
        Rect::new(0, 0, cols, rows),
        state,
        version,
        gs.boot_screen_cols,
    ) else {
        return false;
    };
    let current = (
        gs.machine.mem.read_byte(0x20) as u16,
        gs.machine.mem.read_byte(0x21) as u16,
    );
    if current == (rows.min(255), cols.min(255)) {
        return false;
    }
    session.set_screen_dims(rows, cols);
    true
}

/// Resize a Glulx story to a `(cols, rows)` pane and deliver the Glk Arrange its
/// windows repaint on (SQ-0201). `false` for any other engine.
pub fn resize_glulx(session: &mut dyn Engine, (cols, rows): (u16, u16)) -> bool {
    match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) => {
            gs.resize(cols as u32, rows as u32);
            true
        }
        None => false,
    }
}

/// Set a Glulx story's Glk cell pixel size, `(width, height)`, and deliver the
/// Glk Arrange its graphics windows repaint on (SQ-1598) — the resize-time
/// sibling of `TerminalFacts::glk_cell_px`, for a host whose text cells are
/// not 8×16 changing size live (a proportional-font frontend re-measuring its
/// own font, or a window moved to a different display). Resizes every open
/// graphics canvas to `(window cells) × char_px` and drives the game's own
/// redraw the same way [`resize_glulx`] does for a terminal-size change —
/// there is no separate mechanism. `false` for any other engine, and
/// `state.glk_cell_px` is left untouched on that branch.
///
/// Fractional, matching `TerminalFacts::glk_cell_px` (SQ-1603) — each open
/// canvas rounds its own `cells × char_px` independently rather than sharing
/// one pre-rounded ratio; see `AppGlk::canvas_size`.
///
/// Also carries `char_px` onto [`AppState::glk_cell_px`] (SQ-1601), the same
/// field `host::boot::boot_story` seeds at launch — so a later `@restart`
/// (`host::reset::reset_game`, which re-derives its own `char_px` from that
/// field) re-boots at whatever this call most recently set live, rather than
/// the stale value the session originally booted with.
pub fn set_glk_cell_px(session: &mut dyn Engine, state: &mut AppState, char_px: (f64, f64)) -> bool {
    match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) => {
            gs.set_char_px(char_px);
            state.glk_cell_px = Some(char_px);
            true
        }
        None => false,
    }
}

pub use crate::glulx_session::{GlkLayout, GlkWindowRect};
pub use gvm::glk::GlkScreen;

/// Select (`Some`) or leave (`None`) a design-pixel Glk screen for a Glulx
/// story (SQ-1703): `GlkScreen::design(size_px, text_cell_px)` lays splits out
/// in exact pixels with a fractional, possibly non-square text cell. Call it
/// again with a new text cell to relayout and redraw (the sibling of
/// [`set_glk_cell_px`]); `None` returns to cell mode. `false` for any other
/// engine. Does not touch `state.glk_cell_px`.
pub fn set_glk_design_screen(session: &mut dyn Engine, screen: Option<GlkScreen>) -> bool {
    match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) => {
            gs.set_glk_screen(screen);
            true
        }
        None => false,
    }
}

/// Turn on design-size layout for a Glulx story whose `.cfg` states a design
/// size (SQ-1703 P3, SQ-1707): the story lays out at the design size and the
/// whole frame fills the pane — stretched, or fitted by aspect with a centred
/// letterboxed frame, per [`resolve_glk_fit`]. Sets `state.glk_stretch` (design
/// mode on) and `state.glk_fit` for the renderer and the border icon. Called at
/// boot and again after `@restart` (a fresh session starts in cell mode); the
/// pane resizing needs nothing further — see [`GlulxSession::set_glk_design`].
/// Returns whether design mode is now on. A non-Glulx engine, a story with no
/// design size, or `glk_design = false` (per-game, else the global key) leaves
/// (or returns) cell mode.
pub fn apply_glk_design(session: &mut dyn Engine, state: &mut AppState, game_dir: &std::path::Path) -> bool {
    let pg = crate::styles::PerGameConfig::read(game_dir);
    let design = match state.glk_design.as_ref().and_then(|d| d.size()) {
        Some(size) if crate::glk_cfg::resolve_design_on(pg.glk_design, state.config.glk_design) => Some(size),
        _ => None,
    };
    let fit = crate::glk_cfg::resolve_fit_mode(pg.glk_design_fit, state.config.glk_design_fit);
    let was_design = glk_fit(session).is_some();
    let on = match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) if design.is_some() => {
            gs.set_glk_design_fit(design, fit);
            true
        }
        // Design layout switched OFF on a re-apply (`glk_design = false` while
        // the session is already in design mode): return it to cell layout, and
        // only then, so a cell-mode story sees no extra relayout. `set_glk_design`
        // leaves the mask alone, so it is cleared here too.
        Some(gs) if was_design => {
            gs.set_glk_design_fit(None, fit);
            gs.set_glk_mask(None);
            false
        }
        _ => false,
    };
    state.glk_stretch = on;
    state.glk_fit = fit;
    // P4: the window mask rides with the design state, in design mode only.
    state.glk_mask = match (on, state.glk_design.as_ref().and_then(|d| d.mask_pict)) {
        (true, Some(pict)) => session.as_any_mut().downcast_mut::<GlulxSession>().and_then(|gs| gs.set_glk_mask(Some(pict))),
        _ => None,
    };
    on
}

/// The fit mode this game resolves to: the per-game sidecar's `glk_design_fit`
/// over the global config's (SQ-1707).
pub fn resolve_glk_fit(state: &AppState, game_dir: &std::path::Path) -> crate::glk_cfg::GlkFitMode {
    crate::glk_cfg::resolve_fit_mode(crate::styles::read_per_game_glk_design_fit(game_dir), state.config.glk_design_fit)
}

/// Switch the live fit mode of a design-size Glulx story (SQ-1707): relayouts
/// and redraws, and sets `state.glk_fit`. `false` (nothing changed) when the
/// story is not in design mode (no `.cfg` design size, `glk_design` off, not
/// Glulx). Persisting the choice per game is the caller's business
/// ([`crate::styles::write_per_game_glk_design_fit`]).
pub fn set_glk_fit(session: &mut dyn Engine, state: &mut AppState, mode: crate::glk_cfg::GlkFitMode) -> bool {
    if !state.glk_stretch {
        return false;
    }
    match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) => {
            gs.set_glk_fit_mode(mode);
            state.glk_fit = mode;
            true
        }
        None => false,
    }
}

/// `/set-glk-fit`'s whole effect (SQ-1707), shared by the TUI and any host: pick
/// the mode (`Toggle` flips the one in force, `Auto` clears this game's
/// override and falls back to the global `glk_design_fit`), persist it in the
/// per-game sidecar (`Auto` removes the key), and apply it live. `Ok` is the
/// line to tell the player; `Err` is a refusal that changed nothing — the game
/// has no design size, or design layout is off for it.
pub fn run_set_glk_fit(
    session: &mut dyn Engine,
    state: &mut AppState,
    game_dir: &std::path::Path,
    arg: crate::slash::GlkFitArg,
) -> Result<String, String> {
    use crate::slash::GlkFitArg;
    if state.glk_design.as_ref().and_then(|d| d.size()).is_none() {
        return Err("this game has no design size".into());
    }
    if !state.glk_stretch {
        return Err("design-size layout is off for this game (glk_design = false)".into());
    }
    let want = match arg {
        GlkFitArg::Mode(m) => Some(m),
        GlkFitArg::Toggle => Some(state.glk_fit.toggled()),
        GlkFitArg::Auto => None,
    };
    crate::styles::write_per_game_glk_design_fit(game_dir, want).map_err(|e| format!("set-glk-fit failed: {e}"))?;
    let live = want.unwrap_or(state.config.glk_design_fit);
    if !set_glk_fit(session, state, live) {
        return Err("this game has no design size".into());
    }
    Ok(format!(
        "glk fit: {} (for this game — glk_design_fit = {})",
        want.map_or("auto", |m| m.key()),
        live.key()
    ))
}

/// The [`crate::glk_cfg::GlkFit`] for a Glulx story in design mode — the frame
/// size and offset, each window's rect and the click inverse for the pane as it
/// is now, in cells (SQ-1707). `None` in cell mode or for another engine. A
/// pixel host builds its own with [`crate::glk_cfg::GlkFit::pixels`] and
/// [`glk_design_size`].
pub fn glk_fit(session: &mut dyn Engine) -> Option<crate::glk_cfg::GlkFit> {
    session.as_any_mut().downcast_mut::<GlulxSession>().and_then(|gs| gs.glk_fit())
}

/// The primary text-buffer window id of a Glulx story (SQ-1707), `None` for
/// another engine or before a text buffer is open.
pub fn glk_primary_text_window(session: &mut dyn Engine) -> Option<u32> {
    session.as_any_mut().downcast_mut::<GlulxSession>().and_then(|gs| gs.primary_text_window())
}

/// The design size, in design pixels, a story's `.cfg` states, when design
/// layout applies to this game (a `.cfg` with `WindowWidth` and `WindowHeight`
/// was found and `glk_design` is on, per-game over global); the size a pixel
/// host feeds [`crate::glk_cfg::GlkFit::pixels`].
pub fn glk_design_size(state: &AppState, game_dir: &std::path::Path) -> Option<(u32, u32)> {
    let on = crate::glk_cfg::resolve_design_on(crate::styles::read_per_game_glk_design(game_dir), state.config.glk_design);
    state.glk_design.as_ref().and_then(|d| d.size()).filter(|_| on)
}

/// The Glk screen and the leaf windows' rects (in its layout units) for a
/// Glulx story; `None` for any other engine.
pub fn glk_layout(session: &mut dyn Engine) -> Option<GlkLayout> {
    session.as_any_mut().downcast_mut::<GlulxSession>().map(|gs| gs.glk_layout())
}

/// Tell the story its pane is `(cols, rows)` cells, whatever the engine: the
/// Z-machine header (when it changed) or a Glulx resize. Returns `true` when the
/// story was told something new. A zero-area pane is ignored.
///
/// Honors `state.min_story_screen` (SQ-1596/SQ-1606): when set, `pane` is first
/// bumped up to [`min_story_pane_for_floor`] before either engine call, so a
/// live resize automatically re-derives the same floor boot/`@restart` already
/// apply — a host only has to set `TerminalFacts::min_story_screen` once, at
/// launch, and every later resize honors it with no extra work on its part.
/// `None` (the default) leaves `pane` untouched, exactly as before this floor
/// existed.
pub fn set_story_pane(session: &mut dyn Engine, state: &AppState, pane: (u16, u16)) -> bool {
    if pane.0 == 0 || pane.1 == 0 {
        return false;
    }
    let pane = match state.min_story_screen {
        Some(floor) => min_story_pane_for_floor(session, state, pane, floor),
        None => pane,
    };
    sync_zvm_screen_dims(session, state, pane) | resize_glulx(session, pane)
}

/// The smallest story PANE `(cols, rows)` — at or above `real`, each dimension
/// bumped independently — whose story-facing screen dims clear `floor` in
/// that dimension, computed in PANE space rather than terminal space
/// (SQ-1606).
///
/// [`boot::min_terminal_size_for_story_floor`](crate::host::boot::min_terminal_size_for_story_floor)
/// answers the same question for a host that holds a TERMINAL size and calls
/// it before `compute_pane_layout` runs (boot, `@restart`). A live resize's
/// entry point, [`set_story_pane`], is downstream of that split already — its
/// caller hands it the pane a host's own layout already carved (SQ-1539) — so
/// a host at a live resize has no terminal size to feed the terminal-space
/// search, and would otherwise have to re-derive one, or make a second call
/// just to learn what pane the terminal-space answer yields (exactly the
/// friction `sq1596_min_story_screen_floor.rs`'s
/// `a_host_can_call_the_search_directly_at_a_live_resize` test has to route
/// around).
///
/// Engine-aware, mirroring [`sync_zvm_screen_dims`]/[`resize_glulx`]'s own
/// downcasts, since the chrome a pane crosses before reaching the story
/// differs by engine:
/// - a v4+, non-v6 Z-machine session searches on the RAW
///   [`story_screen_dims`](crate::render::screen::story_screen_dims) for a
///   candidate pane — not the header-floored
///   [`declared_story_screen_dims`](crate::render::screen::declared_story_screen_dims)
///   that [`sync_zvm_screen_dims`] itself applies when it actually WRITES the
///   header — searched one cell at a time via
///   [`boot::bump_dim_for_floor`](crate::host::boot::bump_dim_for_floor). This
///   is deliberate, not an oversight: `declared_story_screen_dims` floors its
///   result at `boot_screen_cols`, and a host that also seeds
///   `TerminalFacts::min_story_screen` at boot has already widened the
///   pre-boot terminal (via
///   [`min_terminal_size_for_story_floor`](crate::host::boot::min_terminal_size_for_story_floor))
///   until the RAW pre-boot pane clears the floor — so `boot_screen_cols`
///   itself ends up at or above `floor_cols` from the moment the session
///   boots. Probing with `declared_story_screen_dims` at a later live resize
///   would then floor EVERY candidate's returned cols at `boot_cols >=
///   floor_cols`, trivially "passing" on the very first candidate regardless
///   of its actual raw width, and this search would return `real` completely
///   unchanged even when the pane's real rendered grid is still narrower than
///   `floor` (a follow-up to SQ-1606, found after the feature first shipped).
///   The header-floor subtraction still belongs in `sync_zvm_screen_dims`,
///   which genuinely must never let a WRITTEN header shrink below
///   `boot_cols` — but the SEARCH here is answering a different question
///   ("what pane makes the raw rendered grid at least this wide"), and must
///   not reuse that clamp as its yardstick;
/// - a v1-3 or v6 session is exempt, matching `declared_story_screen_dims`'s
///   own exemption: no floor applies to either, and `real` is returned
///   unchanged;
/// - a Glulx session has NO chrome subtraction at all — `resize_glulx`
///   delivers the pane verbatim — so clearing the floor is the identity
///   mapping: bump `real` directly against `floor`, no search needed;
/// - neither engine (including a session this crate does not recognize)
///   leaves `real` unchanged.
///
/// A pinned `virtual_screen_cols`/`virtual_screen_rows` wins over the floor in
/// that dimension, exactly as it already wins in
/// `min_terminal_size_for_story_floor` and in `story_screen_dims` itself.
pub fn min_story_pane_for_floor(
    session: &dyn Engine,
    state: &AppState,
    real: (u16, u16),
    floor: (u16, u16),
) -> (u16, u16) {
    let (real_cols, real_rows) = real;
    let (floor_cols, floor_rows) = floor;

    if let Some(gs) = zvm_session_opt(session) {
        let version = gs.machine.mem.version();
        if version < 4 || version == 6 {
            return real;
        }
        let probe = |pane: (u16, u16)| {
            crate::render::screen::story_screen_dims(Rect::new(0, 0, pane.0, pane.1), state)
        };
        let cols = if floor_cols == 0 || state.config.virtual_screen_cols.is_some() {
            real_cols
        } else {
            crate::host::boot::bump_dim_for_floor(real_cols, floor_cols, |candidate| {
                probe((candidate, real_rows.max(1))).map(|(_, cols)| cols)
            })
        };
        let rows = if floor_rows == 0 || state.config.virtual_screen_rows.is_some() {
            real_rows
        } else {
            crate::host::boot::bump_dim_for_floor(real_rows, floor_rows, |candidate| {
                probe((real_cols.max(1), candidate)).map(|(rows, _)| rows)
            })
        };
        return (cols, rows);
    }

    if glulx_session_opt(session).is_some() {
        let cols = if floor_cols == 0 { real_cols } else { real_cols.max(floor_cols) };
        let rows = if floor_rows == 0 { real_rows } else { real_rows.max(floor_rows) };
        return (cols, rows);
    }

    real
}
