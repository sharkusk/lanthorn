//! SQ-0749: `Machine::v6_input_window` was not written into the host archive.
//!
//! `Machine::v6_input_window` (SQ-0585) records which v6 window the game is
//! reading input through, and `BufferWindow::reads_input` (SQ-0746) derives
//! straight from it — that flag is what makes the host draw the player's typed
//! digits into a secondary panel instead of the (silent) primary story window.
//! It was never persisted, so a Restore State taken mid-read through a panel came
//! back with it at 0: the panel's own `reads_input` went false, and the echo of
//! whatever the player typed stayed dark until the next `read` re-established the
//! field. Everything else on the restored frame looked correct — which is why
//! asserting immediately after the restore would not have caught it. The fix
//! moves the field onto `zvm::screen::ScreenState` (alongside `current_fg`/
//! `current_bg`, for the same reason: an input to what the screen must show,
//! archived with the rest of `screen.bin`) so `restore_screen`'s existing
//! wholesale `machine.screen = screen` carries it for free.
//!
//! `stories/fmvpoker.z6` is the known reproducer: choosing "CHANGE CURRENT BET"
//! (SQ-0739) makes the game read through its bottom panel, window 2, instead of
//! window 0. Per the repo's restore-test convention, this test RESTORES, then
//! PERTURBS (types at the restored prompt), then asserts — the same shape as
//! `v6_restore_palette_replay.rs`. Both `honor_game_colours` modes are pinned,
//! and a restore into a different terminal size is covered alongside a same-size
//! one. Skip-if-missing per the other gitignored-story smokes.
//!
//! The assertion renders through [`app::render::v6_layout::draw_secondary_prose`]
//! directly — the one function that reads `reads_input` to place the live echo —
//! rather than the full `build_v6_raster_canvas` pipeline. That fuller pipeline
//! paints the story's declared page over any pixel the game's own erase-fill
//! "painted ground" (`GameSession::paint`, `Engine::paint_surface`) does not
//! cover, and that ground is NOT part of the archive — a separate, pre-existing
//! gap unrelated to this quest (see the note left on SQ-0749): after ANY v6 host
//! Restore, `Engine::paint_surface` comes back `None` until the game's next
//! `erase_window`, so a full-composite diff would fail on fmvpoker's specific
//! panel whether or not `v6_input_window` itself is restored, which is not a
//! meaningful test of this fix. `draw_secondary_prose` alone isolates exactly the
//! one consumer of the field this quest carries through a restore.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;


fn story_path() -> PathBuf {
    fixture_path("fmvpoker.z6")
}

fn picts() -> PictSource {
    PictSource::new(blorb::resolve_resource_blorb(&story_path()).map(|(b, _)| b))
}

/// fmvpoker's bottom panel — window 2 — at native (23,236) 1-based, so (22,235)
/// on the 0-based composite (mirrors `v6_fmvpoker_hybrid.rs`'s `PANEL`).
const PANEL: (u16, u16, u16, u16) = (22, 235, 594, 156);

fn boot(honor: bool) -> Option<(Vec<u8>, GameSession)> {
    let bytes = std::fs::read(story_path()).ok()?;
    let mut pic = picts();
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes.clone(), honor, false, None, false, dims, pic.std_window(), None, None,
    )
    .expect("fmvpoker (v6) boots");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    Some((bytes, s))
}

/// Dismiss the title, then choose "CHANGE CURRENT BET" (key `c`), which hands
/// the read to fmvpoker's bottom panel (SQ-0739) instead of window 0.
fn to_bet_prompt(honor: bool) -> Option<(Vec<u8>, GameSession)> {
    let (bytes, mut session) = boot(honor)?;
    let r = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    assert!(r.fault.is_none(), "fmvpoker faulted dismissing the title: {:?}", r.fault);
    let r = session.submit_char(b'c');
    assert!(r.fault.is_none(), "fmvpoker faulted choosing CHANGE CURRENT BET: {:?}", r.fault);
    Some((bytes, session))
}

fn panel_lines(session: &GameSession) -> Vec<String> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let panel = items
        .iter()
        .find(|it| (it.x_px, it.y_px, it.w_px, it.h_px) == PANEL)
        .expect("fmvpoker publishes its bottom panel as a window of its own");
    let WinNode::Buffer(b) = &panel.node else { panic!("the panel is a prose Buffer") };
    b.lines.clone()
}

/// `draw_secondary_prose` alone, on a blank canvas — the panel's own printed
/// lines plus (SQ-0746) the live input echo of whichever window `reads_input`
/// names. See the module doc for why this is isolated from the full raster
/// pipeline rather than driven through it.
fn secondary_prose_canvas(session: &GameSession, state: &app::state::AppState) -> image::RgbaImage {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let mut canvas = image::RgbaImage::new(native.0.max(1) as u32, native.1.max(1) as u32);
    let ink = image::Rgba([0u8, 0, 0, 255]);
    let panel_input = (state.effective_transcript_scroll() == 0).then_some(state.input.value.as_str());
    app::render::v6_layout::draw_secondary_prose(
        &mut canvas, &layout.chrome, ink, state.config.honor_game_colours, &state.colors, panel_input,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    );
    canvas
}

/// The bounding box of the pixels two canvases disagree on; `None` when identical.
fn diff_box(a: &image::RgbaImage, b: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..a.height().min(b.height()) {
        for x in 0..a.width().min(b.width()) {
            if a.get_pixel(x, y) != b.get_pixel(x, y) {
                (x0, y0) = (x0.min(x), y0.min(y));
                (x1, y1) = (x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0 < x1).then_some((x0, y0, x1, y1))
}

fn app_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

/// Restore, then PERTURB (type at the restored prompt), then assert the echo
/// appears — asserting on the frame immediately after a restore cannot catch
/// this defect, since everything else on it is already correct (SQ-0749).
///
/// `resize` covers a restore into a different terminal size: a resize the game
/// never saw. v6 lays out on its own fixed pixel screen and is deliberately
/// exempt from the host pane feed (`Engine::set_screen_dims` no-ops for v6), so
/// this proves the restored input window is not disturbed by whatever a real
/// resize event triggers on the way in.
fn a_restore_mid_bet_still_echoes_typed_digits(honor: bool, resize: bool) {
    let Some((bytes, session)) = to_bet_prompt(honor) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let lines = panel_lines(&session);
    let row = lines
        .iter()
        .position(|l| l.starts_with("Enter the new bet:"))
        .expect("the bet prompt reached the panel before saving");
    let input_window_before = session.machine.screen.v6_input_window;
    assert_ne!(
        input_window_before, 0,
        "premise: fmvpoker must be reading the bet through its bottom panel, not window 0"
    );

    // Save through the REAL archive Save State path.
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let path = app::scratch_dir(&format!("fmvpoker-input-window-{honor}-{resize}"))
        .join("save.lanthorn");
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(),
            location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &app::archive::SessionRecord::empty(),
        &session.pictures_png(),
        None,
        None,
    )
    .expect("save_archive_meta_pics");
    let ac = app::archive::load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        ac.screen.as_ref().map(|s| s.v6_input_window),
        Some(input_window_before),
        "screen.bin must carry the input window the game was reading through (SQ-0749)"
    );

    // Restore into a completely FRESH session, as Save State / auto-resume do.
    let mut fresh = GameSession::new_with_trace(
        bytes, honor, false, None, false, picts().all_pict_dims(), picts().std_window(), None, None,
    )
    .expect("fresh fmvpoker boot");
    fresh.set_pict_source(Some(picts()));
    fresh.flush_boot_pictures();
    Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore_state");
    app::session::restore_screen(&mut fresh, ac.screen.clone().expect("persisted screen"));
    fresh.load_pictures_png(&ac.pictures);

    if resize {
        Engine::set_screen_dims(&mut fresh, 30, 60);
    }

    let mut state = app_state(honor);

    // PERTURB: type at the restored prompt, then assert — never the bare restored
    // frame, which still looks correct even with the pre-fix defect in place (the
    // panel's own prompt text is unaffected by `v6_input_window`; only the live
    // echo of what the player types depends on it).
    let quiet = secondary_prose_canvas(&fresh, &state);
    state.input.value = "25".into();
    let typed = secondary_prose_canvas(&fresh, &state);

    let (_, y0, _, y1) = diff_box(&quiet, &typed).unwrap_or_else(|| {
        panic!(
            "honor={honor} resize={resize}: typing \"25\" at the restored bet prompt changed \
             NOTHING on the screen. The restored `v6_input_window` is {}, the saved session's was \
             {input_window_before} — an unpersisted field came back as window 0 and the panel's \
             read never re-registered, so its echo went dark (SQ-0749).",
            fresh.machine.screen.v6_input_window
        )
    });
    let top = PANEL.1 as u32 + row as u32 * 16;
    assert!(
        (top..top + 16).contains(&y0) && y1 <= top + 16,
        "honor={honor} resize={resize}: the echo drew at rows {y0}..{y1}, not on the prompt's own \
         row {top}..{}",
        top + 16
    );
}

#[test]
fn a_restore_mid_bet_still_echoes_typed_digits_honoring_game_colours() {
    a_restore_mid_bet_still_echoes_typed_digits(true, false);
}

#[test]
fn a_restore_mid_bet_still_echoes_typed_digits_theme_only() {
    a_restore_mid_bet_still_echoes_typed_digits(false, false);
}

#[test]
fn a_restore_into_a_different_terminal_size_still_echoes_typed_digits_honoring_game_colours() {
    a_restore_mid_bet_still_echoes_typed_digits(true, true);
}

#[test]
fn a_restore_into_a_different_terminal_size_still_echoes_typed_digits_theme_only() {
    a_restore_mid_bet_still_echoes_typed_digits(false, true);
}
