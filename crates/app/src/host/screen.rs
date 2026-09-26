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
/// there is no separate mechanism. `false` for any other engine.
pub fn set_glk_cell_px(session: &mut dyn Engine, char_px: (u32, u32)) -> bool {
    match session.as_any_mut().downcast_mut::<GlulxSession>() {
        Some(gs) => {
            gs.set_char_px(char_px);
            true
        }
        None => false,
    }
}

/// Tell the story its pane is `(cols, rows)` cells, whatever the engine: the
/// Z-machine header (when it changed) or a Glulx resize. Returns `true` when the
/// story was told something new. A zero-area pane is ignored.
pub fn set_story_pane(session: &mut dyn Engine, state: &AppState, pane: (u16, u16)) -> bool {
    if pane.0 == 0 || pane.1 == 0 {
        return false;
    }
    sync_zvm_screen_dims(session, state, pane) | resize_glulx(session, pane)
}
