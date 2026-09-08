//! The frame-to-frame lifecycle of the v6 pixel path's uploaded images — SQ-0747.
//!
//! Three passes chased Journey's truncated menu labels through single frames —
//! 18414 configurations, then 960, then 22000 — and every one came back clean,
//! because **they all rendered one frame from scratch**. The user then established
//! that the defect ACCUMULATES: going to a larger terminal and back truncates more
//! than before. A defect that accumulates is not a property of any frame's layout;
//! it lives in what happens to an image BETWEEN frames.
//!
//! So this file asserts on transitions, not on geometry. It drives a real sequence
//! — Journey's raster title frames, the switch to the hybrid ring, a resize out and
//! a resize back — and holds the pixel path to two rules:
//!
//!   1. **A band whose pixels have not changed must not be re-uploaded.** That is
//!      the SQ-0514 property, and Journey's menu was violating it on every single
//!      frame: the Menu-plan flank draws its art at `menu_flank_panel`'s DEST rect,
//!      the band cache is keyed on the rect a band is drawn at, and only the STRIP
//!      rect was in the live set. `retain_chrome_bands` therefore evicted the panel
//!      every frame — and one eviction clears the whole cache — so all three bands
//!      re-encoded and re-uploaded, forever, for pixels the terminal already had.
//!      The user's `/dump-windows` shows the arithmetic: `band uploads since launch:
//!      78` across `raster x2 · hybrid-ring x27`, i.e. exactly three a frame.
//!
//!   2. **No placement outlives the strip that owns it.** Everything placed on one
//!      frame is either placed again on the next or explicitly dropped. This now
//!      covers the raster→ring transition too: the full-frame composite is placed by
//!      `redraw_v6` and abandoned by `invalidate_v6`, and neither was recorded, so
//!      the one transition Journey's boot actually performs was invisible to every
//!      placement-level harness ever pointed at it.
//!
//! Both `honor_game_colours` modes, per the project's colour-render convention.
//! Kitty cell metrics (8×18) and pane (138×68) are the user's own, from the dump.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::graphics::{kitty_picker, GraphicsOp, GraphicsTarget};
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The user's terminal: 140×71 with nothing else docked, so a 138×68 story pane.
const SMALL: Rect = Rect { x: 1, y: 1, width: 138, height: 68 };
/// "Larger, then back" — the move that made the truncation worse.
const LARGE: Rect = Rect { x: 1, y: 1, width: 170, height: 80 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Journey booted under `profile`, at the frame after boot — deliberately BEFORE
/// the intro is driven, so the caller walks the same raster→ring transition the
/// player's launch does.
fn journey(profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey (v6) should load and boot without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// A hybrid v6 render through a kitty terminal at the user's 8×18 cell. Halfblocks
/// reports 10×20 and draws glyphs, so it never reaches the band cache at all.
fn kitty_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(kitty_picker(8, 18));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

fn frame(session: &GameSession, state: &app::state::AppState, area: Rect) -> Buffer {
    let model = session.screen();
    let mut buf = Buffer::empty(area);
    app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    buf
}

fn advance(session: &mut GameSession) {
    match session.pending_input() {
        InputKind::Line | InputKind::Event => {
            let _ = session.submit("");
        }
        InputKind::Char => {
            let _ = session.submit_char(13);
        }
    }
}

/// Drive Journey to its party/command menu — the reported frame: picture column on
/// the left, prose beside it, the command menu along the bottom.
fn drive_to_menu(session: &mut GameSession, state: &app::state::AppState) {
    for _ in 0..40 {
        let _ = frame(session, state, SMALL);
        let r = match session.pending_input() {
            InputKind::Line | InputKind::Event => session.submit(""),
            InputKind::Char => session.submit_char(13),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
}

fn placed(ops: &[GraphicsOp]) -> std::collections::BTreeSet<GraphicsTarget> {
    ops.iter().filter(|o| o.is_place()).map(|o| o.target()).collect()
}

fn dropped(ops: &[GraphicsOp]) -> std::collections::BTreeSet<GraphicsTarget> {
    ops.iter()
        .filter_map(|o| match o {
            GraphicsOp::Drop { target } => Some(*target),
            _ => None,
        })
        .collect()
}

/// RULE 1 — the one Journey's menu broke on every frame it ever drew.
///
/// Render the same screen twice with nothing changed in between. Every band is
/// already uploaded and every band's pixels are identical, so the second frame must
/// send the terminal nothing at all.
#[test]
fn an_unchanged_menu_frame_re_uploads_no_band() {
    for honor in [true, false] {
        let Some(mut session) = journey(InterpreterProfile::Amiga) else {
            return;
        };
        let state = kitty_state(honor);
        drive_to_menu(&mut session, &state);

        // Two settling frames: the first ring frame after the raster boot is a
        // RESUME (SQ-0587) and re-uploads by design, and the frame after it is the
        // first that can be all cache hits.
        let _ = frame(&session, &state, SMALL);
        let _ = frame(&session, &state, SMALL);
        let before = state.graphics_render.borrow().band_encodes;
        let _ = frame(&session, &state, SMALL);
        let gr = state.graphics_render.borrow();
        assert_eq!(
            gr.band_encodes, before,
            "honor={honor}: an unchanged Journey menu frame re-uploaded {} band(s). \
             The Menu-plan flank panel draws at a rect the live set does not claim, so \
             `retain_chrome_bands` evicts it — and one eviction clears the WHOLE cache. \
             band log:\n{:#?}\nops:\n{:#?}",
            gr.band_encodes - before,
            gr.band_log,
            gr.ops()
        );
        // …and the bands are still on screen: a cache hit that skips its PLACEMENT is
        // the opposite failure (SQ-0587), and this assertion must not be satisfiable
        // by simply drawing nothing.
        assert!(
            placed(gr.ops()).iter().any(|t| matches!(t, GraphicsTarget::Band(..))),
            "honor={honor}: the unchanged frame placed no band at all; ops:\n{:#?}",
            gr.ops()
        );
    }
}

/// RULE 2 — across every transition Journey's launch actually performs.
///
/// Boot through the raster title frames into the hybrid ring, resize larger, resize
/// back. At each step, anything the previous frame put on screen must either be put
/// there again or be explicitly released; an image that is neither is one the
/// terminal still composites and no frame owns.
#[test]
fn no_placement_outlives_the_frame_that_owns_it() {
    for honor in [true, false] {
        let Some(mut session) = journey(InterpreterProfile::Amiga) else {
            return;
        };
        let state = kitty_state(honor);

        // The launch sequence: boot frames at the reported size (Journey's opening
        // goes through the RASTER path before the menu switches to the ring), then
        // out to a larger terminal, then back.
        let plan: Vec<Rect> = std::iter::repeat_n(SMALL, 8)
            .chain(std::iter::repeat_n(LARGE, 3))
            .chain(std::iter::repeat_n(SMALL, 4))
            .collect();

        let mut prev: Option<(usize, Rect, std::collections::BTreeSet<GraphicsTarget>)> = None;
        let mut saw_raster = false;
        let mut saw_ring = false;
        for (step, area) in plan.into_iter().enumerate() {
            let _ = frame(&session, &state, area);
            let gr = state.graphics_render.borrow();
            let now_placed = placed(gr.ops());
            let now_dropped = dropped(gr.ops());
            match state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).as_deref() {
                Some("raster") => saw_raster = true,
                Some("hybrid-ring") => saw_ring = true,
                _ => {}
            }
            if let Some((pstep, parea, pplaced)) = &prev {
                for t in pplaced {
                    assert!(
                        now_placed.contains(t) || now_dropped.contains(t),
                        "honor={honor}: {t:?} was placed on frame {pstep} ({}x{}) and on frame \
                         {step} ({}x{}) it was neither re-placed nor dropped — the terminal is \
                         still compositing an image no frame owns.\nplaced now: {now_placed:?}\n\
                         dropped now: {now_dropped:?}\nops:\n{:#?}",
                        parea.width,
                        parea.height,
                        area.width,
                        area.height,
                        gr.ops()
                    );
                }
            }
            prev = Some((step, area, now_placed));
            drop(gr);
            advance(&mut session);
        }
        assert!(
            saw_raster && saw_ring,
            "honor={honor}: the sequence must cross the raster→ring transition to be \
             testing anything (raster seen: {saw_raster}, ring seen: {saw_ring})"
        );
    }
}

/// The cache-side half of rule 2: every band the cache still holds must have been
/// PLACED on the frame that just finished. A retained key nothing re-places is an
/// upload the terminal keeps and no strip points at — and, because a cache hit sends
/// nothing, it is invisible from the outside. This is what makes an Art strip that
/// is skipped for having no art behind it (`strip_has_art`) a leak rather than a
/// no-op once the cache is allowed to survive a frame at all.
#[test]
fn the_band_cache_holds_only_what_this_frame_placed() {
    for honor in [true, false] {
        let Some(mut session) = journey(InterpreterProfile::Amiga) else {
            return;
        };
        let state = kitty_state(honor);
        for area in [SMALL, SMALL, SMALL, LARGE, LARGE, SMALL, SMALL] {
            let _ = frame(&session, &state, area);
            let gr = state.graphics_render.borrow();
            let live_now = placed(gr.ops());
            for (slot, x, y, w, h) in gr.chrome_band_hashes().keys().copied() {
                assert!(
                    live_now.contains(&GraphicsTarget::Band(x, y, w, h)),
                    "honor={honor}: the band cache holds slot {slot} ({x},{y},{w}x{h}) but \
                     this {}x{} frame never placed it.\nplaced: {live_now:?}\nband log:\n{:#?}",
                    area.width,
                    area.height,
                    gr.band_log
                );
            }
            drop(gr);
            advance(&mut session);
        }
    }
}
