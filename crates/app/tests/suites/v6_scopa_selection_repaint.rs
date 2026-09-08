//! SQ-0788: changing which card is selected must reach the screen.
//!
//! The player reported that selecting a card in scopa outlines it, and selecting a
//! DIFFERENT card then changes nothing at all — the old outline is not erased and
//! the new one never appears — while clicking OK plays the newly-selected card. So
//! the game's model was right throughout and only the picture was stale.
//!
//! The mechanism is the SQ-0469 raster generation gate. `v6_raster_gen` folds every
//! input `build_v6_raster_canvas` reads into one key, and `v6_wants_build` skips the
//! whole rebuild + resize + encode when it has not moved. Its own doc claimed "the
//! model is observed, so no v6 paint or erase can slip past" — true when it was
//! written, and false from SQ-0706 onwards, because the PAINTED GROUND rides beside
//! the window tree rather than inside it.
//!
//! scopa is the game that turns that hole into a visible defect. It draws its whole
//! table with `erase_window` fills (see `v6_scopa_painted_cards.rs`) and publishes no
//! Graphics window at all, so moving the selection outline from one hand card to the
//! next is a ground-only change: measured at 1120 native pixels — the old outline
//! erased, the new one drawn — against a byte-identical window model.
//!
//! That is also why the defect looked intermittent. The FIRST selection always
//! worked: it relabels the confirm button "Choose" -> "OK", a `PxText` change the key
//! already read. And a later selection worked whenever it happened to add or drop a
//! board highlight (the player: "if the new card has a match on the board, the
//! selection will change"), which is a bigger repaint that also moves the model. The
//! outline update was being dropped in every one of those cases too — a neighbouring
//! correct repaint was merely masking it.
//!
//! Falsified by dropping the `v6_paint` arm from `v6_raster_gen`:
//!
//! ```text
//! assertion `left != right` failed: moving the selection outline between two hand
//! cards repaints 1120 native pixels of scopa's painted ground and nothing in its
//! window model — the raster generation key must follow the ground, or
//! `v6_wants_build` skips the rebuild and the player keeps looking at the
//! previously-selected card (SQ-0788)
//!   left: 14596659028018190276
//!  right: 14596659028018190276
//! ```
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


/// The pane the gate is asked about. Any size works — it is hashed, so it only has
/// to be the SAME across the two frames being compared.
const PANE: Rect = Rect { x: 0, y: 0, width: 100, height: 34 };

fn boot() -> Option<GameSession> {
    let path = fixture_path("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// `Engine::set_mouse` takes the coordinates Y FIRST; a click reaches `read_char`
/// as ZSCII 254 (ZMSD §3.8) with the coordinates already set.
fn click(s: &mut GameSession, x: u16, y: u16) {
    Engine::set_mouse(s, y, x);
    let _ = s.submit_char(254);
    let _ = s.take_transcript();
}

fn has_label(s: &GameSession, text: &str) -> bool {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    items.iter().any(|it| match &it.node {
        WinNode::Grid(g) => g.px_texts.iter().any(|t| t.text == text),
        _ => false,
    })
}

/// The app state the raster branch would carry for this frame: the painted ground
/// republished from the engine, exactly as `draw_frame` does before it renders.
fn frame_state(s: &GameSession) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    *state.v6_paint.borrow_mut() = s.paint_surface();
    state
}

/// The generation key `render_story_pane` computes for this frame, and the composite
/// it would build from it.
fn gen_and_composite(s: &GameSession) -> (u64, image::RgbaImage) {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let state = frame_state(s);
    let picker = ratatui_image::picker::Picker::halfblocks();
    let gen = app::render::screen::v6_raster_gen(items, &state, PANE, &picker);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    (gen, canvas)
}

fn pixels_differing(a: &image::RgbaImage, b: &image::RgbaImage) -> usize {
    assert_eq!(a.dimensions(), b.dimensions(), "the two frames are the same screen");
    a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count()
}

/// Boot, pick the Milanese deck off the menu, and play on until the confirm button
/// reads "Choose" — the game asking the player to select a card. (Same route as
/// `v6_scopa_button_labels.rs`.)
fn scopa_at_the_players_turn() -> Option<GameSession> {
    let mut s = boot()?;
    click(&mut s, 250, 350); // the sample Milanese card on the opening menu
    for _ in 0..40 {
        if has_label(&s, "Choose") {
            return Some(s);
        }
        click(&mut s, 590, 380); // the confirm button, to walk past the computer's turn
    }
    panic!("scopa never reached the player's turn — no \"Choose\" label appeared");
}

/// The three hand cards are centred at x = 320 + (idx-1)*44 on the bottom row
/// (`ButtonCard.measure`).
const HAND: [u16; 3] = [276, 320, 364];
const HAND_ROW: u16 = 365;

/// Two DIFFERENT hand cards selected in turn, with the frame each one produces.
/// Returns `None` if this hand offers no second legal card to move to.
fn two_selections(s: &mut GameSession) -> Option<((u64, image::RgbaImage), (u64, image::RgbaImage))> {
    let mut first: Option<(u16, (u64, image::RgbaImage))> = None;
    for x in HAND {
        click(s, x, HAND_ROW);
        if !has_label(s, "OK") {
            continue; // no legal move with that card — nothing was selected
        }
        let frame = gen_and_composite(s);
        match first {
            // A second selectable card whose frame really is a different picture:
            // the pair the player described.
            Some((fx, ref f)) if x != fx && pixels_differing(&f.1, &frame.1) > 0 => {
                return Some((first.take().unwrap().1, frame));
            }
            Some(_) => {}
            None => first = Some((x, frame)),
        }
    }
    None
}

/// The whole defect in one assertion, on the real story.
///
/// Two different cards selected in the same hand paint two different screens, and
/// the raster gate must see that. Before the fix the key was byte-identical across
/// the pair, `v6_wants_build` returned false, and nothing was rebuilt or uploaded —
/// the player's screen kept the first card outlined.
///
/// Walks hands until it finds a pair, because whether a given hand has two legally
/// playable cards is the game's business; it asserts it found one rather than
/// passing vacuously.
///
/// Falsified by dropping the `v6_paint` arm from `v6_raster_gen` — see the module
/// docs for the verbatim failure.
#[test]
fn selecting_a_different_card_repaints_the_screen() {
    let Some(mut s) = scopa_at_the_players_turn() else { return };

    let mut pair = None;
    for _ in 0..12 {
        if has_label(&s, "Choose") {
            if let Some(found) = two_selections(&mut s) {
                pair = Some(found);
                break;
            }
        }
        click(&mut s, 590, 380); // play on: confirm, and walk the computer's turn
    }
    let ((gen_a, frame_a), (gen_b, frame_b)) =
        pair.expect("scopa dealt no hand with two legally selectable cards in twelve tries");

    // The premise: the two selections really are two different pictures.
    let moved = pixels_differing(&frame_a, &frame_b);
    assert!(
        moved > 0,
        "the two selections must paint different screens or there is nothing to detect"
    );

    // The defect: and the gate must know it.
    assert_ne!(
        gen_a, gen_b,
        "moving the selection outline between two hand cards repaints {moved} native pixels of \
         scopa's painted ground and nothing in its window model — the raster generation key must \
         follow the ground, or `v6_wants_build` skips the rebuild and the player keeps looking at \
         the previously-selected card (SQ-0788)"
    );

    // …which is the decision that actually reaches the screen.
    let mut gr = app::render::graphics::GraphicsRender::default();
    let picker = ratatui_image::picker::Picker::halfblocks();
    assert!(gr.v6_wants_build(gen_a, PANE), "the first frame is always built — nothing is cached yet");
    gr.spawn_v6_encode(&picker, frame_a.clone(), gen_a, PANE, app::render::v6_layout::RasterFrame::native((640, 400)));
    assert!(
        !gr.v6_wants_build(gen_a, PANE),
        "the gate's own contract: an unchanged frame is skipped (this is what it is FOR)"
    );
    assert!(
        gr.v6_wants_build(gen_b, PANE),
        "and the second selection must get past it — this is the frame the player never saw"
    );
}

/// The gate's purpose survives the fix: a frame in which nothing at all happened
/// still costs nothing.
///
/// The ground is hashed by content, so republishing the SAME surface — which
/// `draw_frame` does on every single frame — must produce the same key. A key that
/// moved on identity (an `Arc` address, a mutation counter) would rebuild and
/// re-encode scopa's composite on every idle frame instead.
#[test]
fn an_idle_frame_still_skips_the_rebuild() {
    let Some(s) = scopa_at_the_players_turn() else { return };
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let picker = ratatui_image::picker::Picker::halfblocks();

    assert!(s.paint_surface().is_some(), "scopa paints a ground, so this is not a vacuous check");

    // Two independently-built frame states, as two successive `draw_frame` calls
    // would produce them.
    let first = app::render::screen::v6_raster_gen(items, &frame_state(&s), PANE, &picker);
    let second = app::render::screen::v6_raster_gen(items, &frame_state(&s), PANE, &picker);
    assert_eq!(first, second, "an idle frame republishes the same ground and must skip the rebuild");
}

/// A game that paints no ground is unaffected, and a ground appearing or vanishing
/// is itself a change the gate must see (scopa's own full-screen `erase_window`
/// drops the surface — `apply_erase_fill` treats a full-screen fill as a clear).
#[test]
fn an_absent_ground_is_distinguished_from_a_present_one() {
    let Some(s) = scopa_at_the_players_turn() else { return };
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let picker = ratatui_image::picker::Picker::halfblocks();

    let painted = frame_state(&s);
    let with = app::render::screen::v6_raster_gen(items, &painted, PANE, &picker);

    let bare = frame_state(&s);
    *bare.v6_paint.borrow_mut() = None;
    let without = app::render::screen::v6_raster_gen(items, &bare, PANE, &picker);

    assert_ne!(with, without, "losing the painted ground repaints the screen, so it must bump the key");
}
