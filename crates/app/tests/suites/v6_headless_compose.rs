//! SQ-1543: a v6 frame composed by a host that is NOT the TUI.
//!
//! `render::screen::build_v6_raster_frame` used to take `&AppState` and reach into
//! it for colours, the text face, the painted ground and the transcript, so a GUI,
//! a network server or an FFI binding that wanted a Zork Zero frame had to build a
//! whole `AppState` to get one. Composition now reads a
//! [`V6FrameInputs`](app::render::screen::V6FrameInputs), which the TUI builds from
//! its state and any other host builds from its own, and both go through the one
//! [`compose_v6_frame`](app::render::screen::compose_v6_frame).
//!
//! This suite is the host's side of that contract. Every case builds the inputs BY
//! HAND — its own colours, its own face, its own painted ground, its own windowed
//! transcript — and composes without an `AppState`, then checks the result against
//! the canvas the TUI builds for the same frame. It also pins the second half of
//! the quest: the frame's text comes back as DATA (`V6TextRun`s in native pixels),
//! so a host can draw it as real glyphs rather than ship it as pixels (SQ-0750).
//!
//! # The specimens
//!
//! ```text
//!   fixture                  release  turns in  role
//!   zork0-r393-s890714.z6      393        6      a prose frame: ring art, status grid, transcript
//!   journey-r83-s890706.z6      83        6      a chrome-heavy frame: text panel, menu strip
//! ```
//!
//! Each boot goes the way `startup.rs` boots (profile off the MOUNT's medium, then
//! `MachineBoot`) and prints the profile, release, screen, art scale and cell it
//! resolved. Both `honor_game_colours` modes are pinned. The stories are
//! gitignored, so every case skips vacuously without them; the in-crate
//! `glyph_sink_records_the_text_the_draw_images` covers the sink on CI.

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::screen::{compose_v6_frame, RasterMetrics, V6FrameInputs};
use app::render::v6_layout::{self as v6, MainText, RasterFrame, V6TextMode, V6TextRun};
use app::session::{GameSession, InputKind};

/// Taps past the boot art — the frame `v6_raster_reveal` measures Zork Zero at.
const TURNS: usize = 6;

/// The prose the host "wrapped" itself. Short enough that no wrap at any story box
/// on either press splits it, so the TUI's own wrap arrives at the same rows.
const PROSE: [&str; 2] = ["A heavy table stands here in the gloom.", "What next?"];

fn stories_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One boot's facts, travelling together (the `v6_raster_reveal::Booted` shape).
struct Booted {
    session: GameSession,
    face: app::native_font::TextFace,
    palette: zvm::screen::Palette,
    art_scale: (u32, u32),
    honoured: bool,
}

/// Boot the way `startup.rs` boots — see `v6_raster_reveal::boot`, of which this is
/// the same chain for a named fixture and release.
fn boot(file: &str, want_release: u16) -> Option<Booted> {
    let path = stories_dir().join(file);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve_with_override(&path, app::graphics::PictureOverride::Unset, None);
    let dims = picts.all_pict_dims();
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(release, want_release, "{file}: this suite is pinned to release {want_release}");
    let honoured = !picts.declines_game_colours(profile.default_colours());
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source: app::interpreter::ProfileSource::Medium,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        faces.clone(),
        profile.palette(),
        None,
    );
    let art_scale = boot.art_scale;
    let face = app::native_font::TextFace::new(profile, faces, art_scale);
    eprintln!(
        "{file}: booted as {profile:?} off {medium:?} · release {release} · screen {:?} · \
         art_scale {art_scale:?} · v6 cell {:?} · colours {}",
        boot.screen_px,
        face.cell(),
        if honoured { "honoured" } else { "declined" },
    );
    let mut session = GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
        .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..TURNS {
        let t = match session.pending_input() {
            InputKind::Line | InputKind::Event => session.submit("").transcript,
            InputKind::Char => session.submit_char(b' ').transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(Booted { session, face, palette: profile.palette(), art_scale: art_scale.unwrap_or((2, 2)), honoured })
}

/// The TUI's state for the same frame: raster mode, the machine's face and art
/// scale, the painted ground published the way `main.rs` publishes it, and
/// [`PROSE`] as the transcript.
fn tui_state(b: &Booted, honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(b.palette);
    state.config.v6_render = app::config::V6RenderMode::Raster;
    state.config.honor_game_colours = honor;
    state.v6_art_scale = b.art_scale;
    state.v6_text = b.face.clone();
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&b.session);
    for line in PROSE {
        state.push_transcript(line);
    }
    state
}

/// The host's own windowing of its own transcript into a `(cols, rows)` box: every
/// row fits, so it is shown whole from the top with the (empty) input line live.
fn host_prose(cols: u16, rows: u16) -> (MainText, RasterMetrics) {
    let _ = cols;
    let budget = rows.saturating_sub(1);
    let total = PROSE.len() as u16;
    let main = MainText {
        lines: PROSE.iter().map(|s| s.to_string()).collect(),
        styles: Vec::new(),
        input: String::new(),
        cursor_col: 0,
        awaiting: true,
        floats: Vec::new(),
    };
    let metrics = RasterMetrics {
        total_rows: total,
        viewport_rows: budget,
        max_scroll: total.saturating_sub(budget),
        first_visible_row: 0,
    };
    (main, metrics)
}

/// The host's pair, stated the way a host with no theme and no terminal probe
/// would: light grey ink on a black page.
const HOST_INK: image::Rgba<u8> = image::Rgba([220, 220, 220, 255]);
const HOST_PAGE: image::Rgba<u8> = image::Rgba([0, 0, 0, 255]);

/// Compose `b`'s current frame with no `AppState` anywhere: every input is the
/// host's own.
fn host_compose(b: &Booted, honor: bool, text: V6TextMode) -> app::render::screen::V6Frame {
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &b.face);
    let layout = v6::classify_windows(items, b.face.cell());
    let colors = app::colors::ColorScheme::terminal_default_in(b.palette);
    let paint = Engine::paint_surface(&b.session);
    let inputs = V6FrameInputs {
        host_pair: (HOST_INK, HOST_PAGE),
        honor_game_colours: honor,
        colors: &colors,
        face: &b.face,
        paint: paint.as_deref(),
        panel_input: Some(""),
        prose: &host_prose,
        reveal: None,
        pager_active: false,
        more_prompt_pair: (HOST_INK, HOST_PAGE),
        text,
    };
    compose_v6_frame(&layout, RasterFrame::native(native), &inputs)
}

/// The TUI's own composite of the same frame — `build_v6_raster_canvas`, exactly
/// the step `render_story_pane` runs.
fn tui_compose(b: &Booted, honor: bool) -> (image::RgbaImage, Option<RasterMetrics>) {
    let state = tui_state(b, honor);
    // The host's stated pair has to BE the TUI's for this state, or the comparison
    // below is between two different screens.
    assert_eq!(
        app::render::screen::v6_host_pair(&state),
        (HOST_INK, HOST_PAGE),
        "the host pair this suite states is not the TUI's for this state"
    );
    let model = b.session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    app::render::screen::build_v6_raster_canvas(&layout, native, &state)
}

fn specimens() -> Vec<(&'static str, Booted)> {
    [("zork0-r393-s890714.z6", 393), ("journey-r83-s890706.z6", 83)]
        .into_iter()
        .filter_map(|(file, release)| boot(file, release).map(|b| (file, b)))
        .collect()
}

fn run_text(runs: &[V6TextRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("|")
}

/// **The acceptance case.** A host composing from the inputs alone gets the TUI's
/// canvas, pixel for pixel, and the same scroll metrics.
#[test]
fn a_host_composes_the_tuis_canvas_from_the_inputs_alone() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let (tui, tui_metrics) = tui_compose(&b, honor);
            let host = host_compose(&b, honor, V6TextMode::Rasterise);
            eprintln!(
                "{file} honor={honor} (game colours {}): canvas {}x{} · metrics {tui_metrics:?}",
                b.honoured,
                tui.width(),
                tui.height()
            );
            assert_eq!((host.canvas.width(), host.canvas.height()), (tui.width(), tui.height()), "{file} honor={honor}");
            let differing = tui.enumerate_pixels().filter(|&(x, y, p)| host.canvas.get_pixel(x, y) != p).count();
            assert_eq!(differing, 0, "{file} honor={honor}: {differing} pixels differ from the TUI's canvas");
            assert_eq!(host.metrics, tui_metrics, "{file} honor={honor}: scroll metrics");
            assert!(host.text.is_empty(), "Rasterise records nothing");
        }
    }
}

/// The frame's text as data: recording it moves no pixel, and it carries both the
/// host's own prose and the game's own chrome text, at native pixel positions.
#[test]
fn the_frames_text_comes_back_as_runs_without_moving_a_pixel() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let plain = host_compose(&b, honor, V6TextMode::Rasterise);
            let recorded = host_compose(&b, honor, V6TextMode::RasteriseAndRecord);
            assert!(plain.canvas == recorded.canvas, "{file} honor={honor}: recording moved a pixel");
            let all = run_text(&recorded.text);
            eprintln!("{file} honor={honor}: {} runs: {all:.200}", recorded.text.len());
            // Non-vacuity: this is a frame with prose on it, or the case proves
            // nothing about the transcript half.
            assert!(recorded.metrics.is_some(), "{file} honor={honor}: no story box on this frame");
            assert!(all.contains(PROSE[0]), "{file} honor={honor}: the host's prose is not among the runs: {all}");
            let cell = b.face.cell();
            for r in &recorded.text {
                assert_eq!(r.boxes.len(), r.text.chars().count(), "{file}: one box per character in {r:?}");
                assert_eq!(r.h, u32::from(cell.h()), "{file}: a run's box is the text cell's height: {r:?}");
                assert!(r.boxes.windows(2).all(|w| w[0].0 < w[1].0), "{file}: boxes step rightward: {r:?}");
            }
            // …and at least one run the GAME printed, not the host.
            assert!(
                recorded.text.iter().any(|r| !PROSE.iter().any(|p| p.contains(r.text.trim())) && !r.text.trim().is_empty()),
                "{file} honor={honor}: no chrome text was recorded: {all}"
            );
        }
    }
}

/// `RecordOnly` leaves the text to the host: the canvas differs from the
/// rasterised one ONLY inside the recorded glyph boxes — so no glyph was imaged
/// that the run list does not account for — and does differ.
#[test]
fn record_only_images_no_glyph_the_runs_do_not_account_for() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let full = host_compose(&b, honor, V6TextMode::RasteriseAndRecord);
            let bare = host_compose(&b, honor, V6TextMode::RecordOnly);
            assert_eq!(full.text, bare.text, "{file} honor={honor}: both modes see the same text");
            let inside = |x: u32, y: u32| {
                bare.text.iter().any(|r| {
                    (r.y..r.y + r.h).contains(&y) && r.boxes.iter().any(|&(bx, w)| (bx..bx + w).contains(&x))
                })
            };
            let mut differing = 0usize;
            for (x, y, p) in full.canvas.enumerate_pixels() {
                if bare.canvas.get_pixel(x, y) != p {
                    differing += 1;
                    assert!(inside(x, y), "{file} honor={honor}: pixel ({x},{y}) changed outside every glyph box");
                }
            }
            eprintln!("{file} honor={honor}: {differing} glyph pixels left to the host");
            assert!(differing > 0, "{file} honor={honor}: RecordOnly imaged the text anyway");
        }
    }
}
