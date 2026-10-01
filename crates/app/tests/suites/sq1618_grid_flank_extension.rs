//! SQ-1618 — a v6 Hybrid frame's side flank art must extend down a pane taller
//! than the game's own screen even when the story slot is a `Grid`
//! (InvisiClues topic menus — Zork Zero, Shogun), not only a `Buffer`.
//!
//! # The bug
//!
//! The terminal's own Hybrid renderer tiles the side-art flanks down the
//! WHOLE pane regardless of the story slot's node type
//! (`build_hybrid_frame_with`'s `tiled_flanks`, `screen.rs` — the only
//! exclusion there is `BottomPlan::Menu`, Journey's shape; no `WinNode` check
//! at all). But `compose_v6_frame`/`compose_v6_frame_into`'s `ext` block used
//! to decline the WHOLE frame the moment the story slot was not a `Buffer` —
//! the SQ-1026 guard, reused for a question it was never meant to answer
//! ("does this frame extend at all", not "is there a transcript to grow") —
//! so a host composing a raster frame at a pane taller than the game's own
//! screen got a canvas truncated to the game's NATIVE height
//! (`V6Frame.frame.canvas_h == native.1`) on the hint/topic menu screen, even
//! though the terminal's own Hybrid render tiles the flank art the full pane
//! height on the identical frame.
//!
//! # The fix
//!
//! `V6FrameInputs::extend_flanks_under_story_grid` (opt-in, default `false`,
//! every existing caller unaffected) lets a `Grid` story slot's frame extend
//! anyway — `extend_raster_flanks` itself never inspects `story.node` (only
//! `story.x_px`/`w_px` and `frame.canvas_h`), so it already draws the tiled
//! flank art correctly once it is handed a tall enough `frame`; the only
//! thing standing in the way was the upstream guard forcing `extension = 0`.
//! The SQ-1026 guard at its REAL site — declining to draw a transcript into
//! the Grid's rect — is untouched and still applies at every extension level;
//! this suite's own case 3 below is the falsifiable proof of that.
//!
//! # Specimens
//!
//! Reuses `v6_hint_menu_mouse.rs`'s own proven boot sequence (`hint`, then
//! `y`) rather than re-deriving it (CLAUDE.md): both titles that share this
//! one hint/InvisiClues screen shape (SQ-0934) — Zork Zero and Shogun — are
//! driven, because a defect that shows on one must be looked for on the
//! other.
//!
//! | fixture                   | release | to the menu                        |
//! |----------------------------|---------|-------------------------------------|
//! | `zork0-r393-s890714.z6`   | 393     | `hint`, then `y` — 2 inputs         |
//! | `shogun-r322-s890706.z6`  | 322     | Enter past the splash, `hint`, `y`  |
//!
//! Skip-if-missing (gitignored stories, symlink `stories/` into a worktree
//! per CLAUDE.md), and non-vacuous: present fixtures that yield no check fail
//! rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::render::screen::{compose_v6_frame, RasterMetrics, V6FrameInputs};
use app::render::v6_layout::{self as v6, MainText, RasterFrame, V6TextMode};
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a press to its hint/InvisiClues topic menu, or `None` when the
/// gitignored story is absent — `v6_hint_menu_mouse.rs`'s own `hint_menu`,
/// reused rather than re-derived.
fn hint_menu(file: &str, release: u16) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let bytes = std::fs::read(&path).ok()?;
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{file} is not the pinned release");
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
        .expect("the press should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    // Zork Zero asks for a LINE first; Shogun holds a title splash on a CHAR read.
    // Answer whatever is in the way rather than assuming either.
    for _ in 0..8 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    s.submit("hint");
    let entered = s.submit_char(b'y');
    assert!(entered.fault.is_none(), "{file}: entering the hint menu faulted: {:?}", entered.fault);
    Some(s)
}

const SPECIMENS: &[(&str, u16)] = &[("zork0-r393-s890714.z6", 393), ("shogun-r322-s890706.z6", 322)];

const HOST_INK: image::Rgba<u8> = image::Rgba([220, 220, 220, 255]);
const HOST_PAGE: image::Rgba<u8> = image::Rgba([0, 0, 0, 255]);

fn empty_prose(_cols: u16, rows: u16) -> (MainText, RasterMetrics) {
    (
        MainText { lines: Vec::new(), styles: Vec::new(), input: String::new(), cursor_col: 0, awaiting: false, floats: Vec::new() },
        RasterMetrics { total_rows: 0, viewport_rows: rows, max_scroll: 0, first_visible_row: 0, top_anchored_fits: false },
    )
}

/// Compose `session`'s current frame at `want`, at a bare host cell
/// (`zvm::screen::V6Cell::DEFAULT`, matching `v6_journey_menu_band.rs`'s own
/// `menu_anchor_compose` pattern — no `AppState`, every input the host's
/// own), with `extend_flanks_under_story_grid` set as asked.
fn compose(session: &GameSession, want: RasterFrame, extend_flanks_under_story_grid: bool) -> app::render::screen::V6Frame {
    let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let colors = app::colors::ColorScheme::terminal_default();
    let layout = v6::classify_windows(items, tf.cell());
    let inputs = V6FrameInputs {
        host_pair: (HOST_INK, HOST_PAGE),
        honor_game_colours: true,
        colors: &colors,
        face: &tf,
        paint: None,
        panel_input: None,
        input: None,
        prose: &empty_prose,
        reveal: None,
        pager_active: false,
        more_prompt_pair: (HOST_INK, HOST_PAGE),
        text: V6TextMode::RasteriseAndRecord,
        bottom_anchor_menu: false,
        hybrid_text_rows: std::collections::HashSet::new(),
        extend_flanks_under_story_grid,
    };
    compose_v6_frame(&layout, want, &inputs)
}

/// A pane device-pixel size taller than the game's native 640x400 screen —
/// SQ-1618's own repro shape (99x50 host cells at a 20x44px device cell:
/// `(99*20, 50*44)` == `(1980, 2200)`).
const TALL_PANE_DEV: (u32, u32) = (1980, 2200);

/// The acceptance case: a Grid story slot's flanks extend when opted in,
/// don't when not, and the SQ-1026 no-transcript guard survives either way.
#[test]
fn grid_story_slot_extends_flanks_when_opted_in() {
    let mut ran = 0;
    for &(file, release) in SPECIMENS {
        let Some(session) = hint_menu(file, release) else {
            eprintln!("SKIP: gitignored story missing at {}", stories_dir().join(file).display());
            continue;
        };
        ran += 1;

        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{file}: a v6 frame has a Layered root") };
        let cell = zvm::screen::V6Cell::DEFAULT;
        let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(cell));
        let layout = v6::classify_windows(items, cell);

        // Premise: the hint screen's story slot really is a Grid (SQ-1026's
        // own specimen), not a Buffer — the case this quest is about.
        assert!(
            matches!(layout.story.map(|s| &s.node), Some(WinNode::Grid(_))),
            "{file}: premise — the hint screen's story slot is not a Grid on this frame"
        );
        let story = layout.story.expect("a story window");
        let gfx = v6::build_graphics_canvas(&layout.chrome, native);

        let want = RasterFrame::extended(native, TALL_PANE_DEV, cell, Some(2.0), true);
        assert!(want.extension() > 0, "{file}: premise — the tall pane must actually afford an extension");

        // 2. Flag NOT set (every existing caller's default): zero behaviour
        // change — the canvas stays at the game's own native height.
        let off = compose(&session, want, false);
        assert_eq!(
            off.frame.canvas_h,
            u32::from(native.1),
            "{file}: with extend_flanks_under_story_grid left off, the canvas must stay at the \
             game's native height — a Grid story slot must decline exactly as it always has"
        );

        // 1. Flag SET: the frame extends to (matches) `want`, and the newly
        // extended flank pixels agree with the same v6_border tiling recipe
        // extend_raster_flanks and the terminal's own Hybrid tiled_flanks
        // both call (flank_source/art_extent) — the real regression check.
        let on = compose(&session, want, true);
        assert_eq!(
            on.frame.canvas_h, want.canvas_h,
            "{file}: with extend_flanks_under_story_grid set, the frame must extend to the pane's \
             own requested height"
        );
        assert!(
            on.frame.canvas_h > u32::from(native.1),
            "{file}: extend_flanks_under_story_grid must actually grow the canvas past native"
        );

        let native_h = u32::from(native.1);
        let right = (story.x_px as u32).saturating_add(story.w_px as u32);
        let mut checked = 0;
        for (x0, x1) in [(0u32, (story.x_px as u32).min(native.0 as u32)), (right.min(native.0 as u32), native.0 as u32)] {
            if x1 <= x0 {
                continue;
            }
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            // Built purely from the art-only canvas, at the FRAME's own taller
            // height — the same recipe `extend_raster_flanks` and the
            // terminal's `flank_tiled_source` both call.
            let Some(want_flank) = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native_h, 0, on.frame.canvas_h) else {
                continue;
            };
            for y in art.1..on.frame.canvas_h.min(want_flank.height()) {
                for x in 0..want_flank.width().min(on.canvas.width().saturating_sub(x0)) {
                    let w = want_flank.get_pixel(x, y).0;
                    if w[3] < 128 {
                        continue;
                    }
                    let got = on.canvas.get_pixel(x0 + x, y).0;
                    assert_eq!(
                        got, w,
                        "{file}: extended flank pixel ({},{}) disagrees with the tiling recipe \
                         extend_raster_flanks and the terminal's own Hybrid tiled_flanks both use \
                         — canvas_h={}, native_h={native_h}",
                        x0 + x,
                        y,
                        on.frame.canvas_h,
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "{file}: no extended flank pixel was checked — premise failed");

        // 3. SQ-1026 is not reintroduced: no host transcript/prose is drawn
        // into the Grid's rect at any extension level. The guard at its REAL
        // site (main body of compose_v6_frame_into,
        // `!matches!(layout.story.map(|s| &s.node), Some(WinNode::Buffer(_)))`)
        // is untouched, and this is its observable effect: no story box, no
        // scroll metrics, and no glyph run drawn at or past the game's own
        // native screen height (there is no prose to grow into the gap the
        // extension opened).
        assert!(on.story.is_none(), "{file}: a Grid story slot must report no prose box, extended or not");
        assert!(on.metrics.is_none(), "{file}: a Grid story slot must report no scroll metrics");
        for run in &on.text {
            assert!(
                run.y < native_h,
                "{file}: unexpected text run at native y={} (>= the game's own screen height \
                 {native_h}) — SQ-1026 would be reintroduced if a transcript leaked into the \
                 extension",
                run.y
            );
        }
    }
    if stories_dir().join(SPECIMENS[0].0).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}
