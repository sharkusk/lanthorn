//! SQ-1187: the hybrid frame GATE — the falsification half of the quest.
//!
//! The hybrid arm computes a generation key (`v6_hybrid_gen`) over every input
//! the ring compute reads, and replays the cached `HybridFrame` when it holds
//! still. This repo's signature defect is the omitted fact producing a
//! self-consistent wrong screen, so each case here perturbs exactly ONE input
//! and asserts the frame REBUILT (via the `hybrid_builds` counter — the build
//! seam planted for exactly this suite), while the anchor case asserts an
//! untouched frame does NOT rebuild and comes out cell-for-cell identical to
//! the frame it replays.
//!
//! Boots the real Zork0 (gitignored story — skips cleanly when absent),
//! exactly like `v6_hybrid_zork0.rs`.
//!
//! **Palette: `Standard`, under the shared lock** — cases here boot a press and
//! render painted surfaces, and one of them swaps the palette mid-case, which
//! is only legal under the guard (SQ-0987).

use std::path::PathBuf;

use app::engine::{Engine, ScreenModel, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork0 through boot + boot-picture flush; `None` (with a SKIP note) when
/// the gitignored story is absent.
fn boot_zork0() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

fn render_state() -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state
}

fn builds(state: &app::state::AppState) -> u64 {
    state.graphics_render.borrow().hybrid_builds
}

fn render(model: &ScreenModel, state: &app::state::AppState, area: Rect) -> Buffer {
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
    buf
}

/// The frame must actually be the hybrid RING — a takeover or a fall-through to
/// raster would make every "did not rebuild" assertion below vacuous.
fn assert_ring(state: &app::state::AppState) {
    assert!(state.v6_hybrid_ring.get(), "this suite's frame must take the hybrid ring path");
}

fn buffers_equal(a: &Buffer, b: &Buffer, area: Rect) -> bool {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let (ca, cb) = (a.cell((x, y)), b.cell((x, y)));
            match (ca, cb) {
                (Some(ca), Some(cb)) => {
                    if ca.symbol() != cb.symbol() || ca.style() != cb.style() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

#[test]
fn an_unchanged_frame_replays_without_a_rebuild() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    let mut state = render_state();
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);

    let first = render(&model, &state, area);
    assert_ring(&state);
    let after_first = builds(&state);
    assert!(after_first >= 1, "the first frame computes");

    let second = render(&model, &state, area);
    assert_eq!(builds(&state), after_first, "an untouched frame must NOT recompute the ring");
    assert!(
        buffers_equal(&first, &second, area),
        "a replayed frame must be cell-for-cell identical to the frame it replays"
    );
}

/// The story viewport stays LIVE on a replay: new transcript output reaches the
/// screen without the ring recomputing — the whole point of splitting the
/// compute half from the draw half.
#[test]
fn transcript_output_draws_live_without_a_rebuild() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    let mut state = render_state();
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);

    let first = render(&model, &state, area);
    assert_ring(&state);
    let after_first = builds(&state);

    state.push_transcript("A fresh line the ring never saw.");
    let second = render(&model, &state, area);
    assert_eq!(builds(&state), after_first, "transcript output is the draw half's; the ring must not rebuild");
    assert!(
        !buffers_equal(&first, &second, area),
        "the new transcript line must nevertheless reach the screen"
    );
}

/// One case per perturbed input. Each drives the SAME state through a baseline
/// frame, perturbs exactly one input, and asserts the ring recomputed.
fn perturbation_rebuilds(
    perturb: impl FnOnce(&mut ScreenModel, &mut app::state::AppState),
    what: &str,
) {
    let Some(session) = boot_zork0() else { return };
    let mut model = session.screen();
    let mut state = render_state();
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);

    let _ = render(&model, &state, area);
    assert_ring(&state);
    let baseline = builds(&state);
    // Prove the baseline is stable before perturbing, so the +1 below can only
    // come from the perturbation.
    let _ = render(&model, &state, area);
    assert_eq!(builds(&state), baseline, "baseline must replay before the perturbation");

    perturb(&mut model, &mut state);
    let _ = render(&model, &state, area);
    assert!(
        builds(&state) > baseline,
        "{what} must rebuild the hybrid frame (builds stuck at {baseline})"
    );
}

#[test]
fn a_game_paint_rebuilds() {
    // A graphics window's version stamp is how the model says "the game drew":
    // bump one exactly as a paint would.
    perturbation_rebuilds(
        |model, _| {
            let WinNode::Layered(items) = &mut model.root else { panic!("v6 root is layered") };
            let g = items
                .iter_mut()
                .find_map(|pw| match &mut pw.node {
                    WinNode::Graphics(g) => Some(g),
                    _ => None,
                })
                .expect("Zork0's frame has a graphics window");
            g.version = g.version.wrapping_add(1);
        },
        "a game paint (graphics version bump)",
    );
}

#[test]
fn a_status_text_change_rebuilds() {
    perturbation_rebuilds(
        |model, _| {
            let WinNode::Layered(items) = &mut model.root else { panic!("v6 root is layered") };
            let g = items
                .iter_mut()
                .find_map(|pw| match &mut pw.node {
                    WinNode::Grid(g) if !g.px_texts.is_empty() => Some(g),
                    _ => None,
                })
                .expect("Zork0's banner carries grid runs");
            g.px_texts[0].text.push('!');
        },
        "a chrome text change",
    );
}

#[test]
fn a_hidden_window_rebuilds() {
    perturbation_rebuilds(
        |model, _| {
            let WinNode::Layered(items) = &mut model.root else { panic!("v6 root is layered") };
            let gone = items
                .iter()
                .position(|pw| matches!(&pw.node, WinNode::Graphics(_)))
                .expect("Zork0's frame has a graphics window to hide");
            items.remove(gone);
        },
        "a window disappearing",
    );
}

#[test]
fn a_resize_rebuilds() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    let mut state = render_state();
    state.push_transcript("West of House");

    let _ = render(&model, &state, Rect::new(0, 0, 80, 30));
    assert_ring(&state);
    let baseline = builds(&state);
    let _ = render(&model, &state, Rect::new(0, 0, 100, 34));
    assert!(builds(&state) > baseline, "a pane resize must rebuild the hybrid frame");
}

#[test]
fn a_palette_swap_rebuilds() {
    perturbation_rebuilds(
        // SQ-1393: the MACHINE's table is a field on the scheme being rendered
        // now, so a swap is a change to the state the key is computed from —
        // which is the whole point of hashing it.
        |_, state| state.colors.machine_palette = zvm::screen::Palette::Amiga,
        "a palette swap",
    );
}

#[test]
fn a_theme_palette_change_rebuilds() {
    perturbation_rebuilds(
        |_, state| {
            // The theme's ANSI palette is an input the canvas resolves the odd
            // Standard colour through; flipping one slot is a theme change that
            // leaves the default pair alone.
            state.colors.palette[3] = ratatui::style::Color::Rgb(1, 2, 3);
        },
        "a theme palette change",
    );
}

#[test]
fn a_game_colours_toggle_rebuilds() {
    perturbation_rebuilds(
        |_, state| state.config.honor_game_colours = !state.config.honor_game_colours,
        "a /set-game-colours toggle",
    );
}

#[test]
fn a_painted_ground_change_rebuilds() {
    perturbation_rebuilds(
        |_, state| {
            // scopa's whole defect class (SQ-0788): drawing that lands ONLY in
            // the painted ground, no window moved.
            let mut ground = image::RgbaImage::new(640, 400);
            ground.put_pixel(300, 200, image::Rgba([200, 40, 40, 255]));
            *state.v6_paint.borrow_mut() = Some(std::sync::Arc::new(ground));
        },
        "a painted-ground change",
    );
}
