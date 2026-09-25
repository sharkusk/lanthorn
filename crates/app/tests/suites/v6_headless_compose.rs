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
//!   Journey - The Quest         30   6, save,    the Amiga floppy (serial 890322): a machine
//!     Begins.adf                   restore, +1   page pair, composed from the model (SQ-1566)
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
use app::render::screen::{compose_v6_frame, RasterMetrics, V6BottomPlan, V6FrameInputs};
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
    profile: InterpreterProfile,
    face: app::native_font::TextFace,
    palette: zvm::screen::Palette,
    art_scale: (u32, u32),
    honoured: bool,
}

/// Boot the way `startup.rs` boots — see `v6_raster_reveal::boot`, of which this is
/// the same chain for a named fixture and release — then tap [`TURNS`] times.
fn boot(file: &str, want_release: u16) -> Option<Booted> {
    let mut b = boot_at(file, want_release)?;
    for _ in 0..TURNS {
        tap(&mut b.session);
    }
    Some(b)
}

/// One move: an empty line or a space, whichever the game is waiting for, and `n`
/// to any yes-or-no question.
fn tap(session: &mut GameSession) {
    let t = match session.pending_input() {
        InputKind::Line | InputKind::Event => session.submit("").transcript,
        InputKind::Char => session.submit_char(b' ').transcript,
    };
    if t.to_lowercase().contains("y or n") {
        let _ = session.submit_char(b'n');
    }
}

/// [`boot`] without the taps: the boot frame.
fn boot_at(file: &str, want_release: u16) -> Option<Booted> {
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
    Some(Booted { session, profile, face, palette: profile.palette(), art_scale: art_scale.unwrap_or((2, 2)), honoured })
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
    host_compose_with(b, honor, text, &host_prose)
}

/// [`host_compose`] with the host's prose callback named by the caller.
fn host_compose_with(
    b: &Booted,
    honor: bool,
    text: V6TextMode,
    prose: &dyn Fn(u16, u16) -> (MainText, RasterMetrics),
) -> app::render::screen::V6Frame {
    host_compose_full(b, honor, text, prose, Some(""))
}

/// [`host_compose_with`], with the input override (SQ-1567 addendum) named by
/// the caller too — `panel_input` is built from the SAME value, matching how
/// `V6FrameInputs`'s own builder derives it: the story box and a reading panel
/// cannot disagree about the host's draft.
fn host_compose_full(
    b: &Booted,
    honor: bool,
    text: V6TextMode,
    prose: &dyn Fn(u16, u16) -> (MainText, RasterMetrics),
    input: Option<&str>,
) -> app::render::screen::V6Frame {
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
        panel_input: input,
        input,
        prose,
        reveal: None,
        pager_active: false,
        more_prompt_pair: (HOST_INK, HOST_PAGE),
        text,
        bottom_anchor_menu: false,
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
/// rasterised one ONLY inside the recorded glyph boxes and the reported caret
/// cell (SQ-1567) — so nothing was left out that the frame does not account for —
/// and does differ.
#[test]
fn record_only_images_no_glyph_the_runs_do_not_account_for() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let full = host_compose(&b, honor, V6TextMode::RasteriseAndRecord);
            let bare = host_compose(&b, honor, V6TextMode::RecordOnly);
            assert_eq!(full.text, bare.text, "{file} honor={honor}: both modes see the same text");
            assert_eq!(full.caret, bare.caret, "{file} honor={honor}: both modes see the same caret");
            let cell_w = u32::from(b.face.cell().w());
            let inside = |x: u32, y: u32| {
                bare.text.iter().any(|r| {
                    (r.y..r.y + r.h).contains(&y) && r.boxes.iter().any(|&(bx, w)| (bx..bx + w).contains(&x))
                }) || bare.caret.is_some_and(|c| (c.x..c.x + cell_w).contains(&x) && (c.y..c.y + c.h).contains(&y))
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

// ── what the composite measured (SQ-1567) ────────────────────────────────────

/// [`host_prose`]'s lines with `awaiting` set as asked — the frame with and
/// without a live input caret.
fn prose_awaiting(awaiting: bool) -> impl Fn(u16, u16) -> (MainText, RasterMetrics) {
    move |cols, rows| {
        let (mut main, metrics) = host_prose(cols, rows);
        main.awaiting = awaiting;
        (main, metrics)
    }
}

/// A host whose transcript is EMPTY: no line, no live input. Only the page is left
/// inside the story box.
fn empty_prose(_cols: u16, rows: u16) -> (MainText, RasterMetrics) {
    let main = MainText {
        lines: Vec::new(),
        styles: Vec::new(),
        input: String::new(),
        cursor_col: 0,
        awaiting: false,
        floats: Vec::new(),
    };
    let metrics = RasterMetrics { total_rows: 0, viewport_rows: rows, max_scroll: 0, first_visible_row: 0 };
    (main, metrics)
}

/// `story` is the box the prose callback was ASKED to fill: its grid is the
/// callback's own arguments, captured as it was called, and its pixels lie inside
/// the canvas and hold that grid.
#[test]
fn the_story_box_is_the_one_the_prose_callback_was_asked_to_fill() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let asked = std::cell::Cell::new(None);
            let prose = |cols: u16, rows: u16| {
                asked.set(Some((cols, rows)));
                host_prose(cols, rows)
            };
            let f = host_compose_with(&b, honor, V6TextMode::RecordOnly, &prose);
            let (cols, rows) = asked.get().unwrap_or_else(|| panic!("{file} honor={honor}: prose never asked for"));
            let s = f.story.unwrap_or_else(|| panic!("{file} honor={honor}: prose was asked for but no story box reported"));
            eprintln!("{file} honor={honor}: story {s:?} · canvas {}x{}", f.canvas.width(), f.canvas.height());
            assert_eq!((s.cols, s.rows), (cols, rows), "{file} honor={honor}: the grid the callback received");
            assert!(s.w > 0 && s.h > 0, "{file} honor={honor}: a degenerate story box {s:?}");
            assert!(
                s.x + s.w <= f.canvas.width() && s.y + s.h <= f.canvas.height(),
                "{file} honor={honor}: story box {s:?} leaves the {}x{} canvas",
                f.canvas.width(),
                f.canvas.height()
            );
            let cell = b.face.cell();
            assert!(
                u32::from(s.cols) * u32::from(cell.w()) <= s.w && u32::from(s.rows) * u32::from(cell.h()) <= s.h,
                "{file} honor={honor}: the {}x{} grid does not fit in {s:?}",
                s.cols,
                s.rows
            );
        }
    }
}

// ---------------------------------------------------------------------------
// SQ-1566: the machine's own page pair, derived from the model alone.
//
// `V6FrameInputs::from_state` takes the machine pair from `AppState::v6_page_pair`,
// a cell only `render_story_pane` writes. A host that never rendered to the
// terminal therefore composed Journey's Amiga frame on its own default pair
// instead of the machine's white on medium grey. `V6FrameInputs::for_model` reads
// the pair off the model (`screen::v6_machine_pair`), and these cases pin it.
//
//   fixture                         release  serial  profile  turns in
//   Journey - The Quest Begins.adf     30    890322   Amiga   6, save, restore, +1
// ---------------------------------------------------------------------------

const AMIGA_JOURNEY: &str = "Journey - The Quest Begins.adf";

/// Journey r30 off its Amiga floppy: [`TURNS`] taps in, saved, restored into a
/// FRESH boot the way the app restores (engine, screen, display list, ground), and
/// then one more move — a restore defect surfaces on the next repaint, not on the
/// restore itself.
fn amiga_journey_after_restore() -> Option<Booted> {
    let mut played = boot(AMIGA_JOURNEY, 30)?;
    assert!(
        matches!(played.profile, InterpreterProfile::Amiga),
        "{AMIGA_JOURNEY} must boot as the Amiga, got {:?}",
        played.profile
    );
    let es = Engine::save_state(&played.session);
    let screen = played.session.machine.screen.clone();
    let (dto, fallback, _diags) = played.session.display_list();
    let pics = played.session.pictures_png_for(&fallback);
    let ground = played.session.paint_ground_png();

    let mut fresh = boot_at(AMIGA_JOURNEY, 30)?;
    Engine::restore_state(&mut fresh.session, &es).expect("restore");
    app::session::restore_screen(&mut fresh.session, screen);
    fresh.session.load_display_list(&dto, &pics);
    fresh.session.load_paint_ground(ground.as_deref());
    tap(&mut fresh.session);
    Some(fresh)
}

/// Compose `model`'s frame the way a host with no terminal render does: inputs
/// from [`V6FrameInputs::for_model`], nothing read from a render-time cell.
fn for_model_compose(model: &app::engine::ScreenModel, state: &app::state::AppState) -> app::render::screen::V6Frame {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let paint = state.v6_paint.borrow();
    let prose = |cols: u16, rows: u16| app::render::screen::build_main_text(state, cols, rows);
    let inputs = V6FrameInputs::for_model(state, model, paint.as_deref(), &prose, Some(state.input.value.as_str()));
    compose_v6_frame(&layout, RasterFrame::native(native), &inputs)
}

/// The TUI's own `build_v6_raster_canvas` for `model` on `state`.
fn tui_canvas(model: &app::engine::ScreenModel, state: &app::state::AppState) -> (image::RgbaImage, Option<RasterMetrics>) {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    app::render::screen::build_v6_raster_canvas(&layout, native, state)
}

fn differing(a: &image::RgbaImage, b: &image::RgbaImage) -> usize {
    assert_eq!(a.dimensions(), b.dimensions(), "canvas sizes differ");
    a.enumerate_pixels().filter(|&(x, y, p)| b.get_pixel(x, y) != p).count()
}

/// **The acceptance case.** On the Amiga floppy, a host that builds its inputs
/// from the model alone gets the TUI's canvas pixel for pixel in both honour
/// modes — and without SQ-1566 it did not.
///
/// The TUI side is `build_v6_raster_canvas` on the state `render_story_pane` leaves
/// behind: its `v6_page_pair` cell holding what the render published, which
/// [`the_terminal_render_publishes_the_pair_the_model_states`] pins independently.
/// The host side never touches that cell.
///
/// Non-vacuity: with colours honoured, the cell-less `from_state` path — the
/// reported defect — composes a DIFFERENT canvas.
#[test]
fn a_host_composes_the_amiga_journey_frame_from_the_model_alone() {
    let Some(b) = amiga_journey_after_restore() else { return };
    assert!(b.honoured, "{AMIGA_JOURNEY}: the Amiga press honours game colours, or this case is vacuous");
    assert_eq!(b.art_scale, (2, 2), "{AMIGA_JOURNEY}: the Amiga's 320x200 picture space doubles");
    assert_eq!((b.face.cell().w(), b.face.cell().h()), (8, 16), "{AMIGA_JOURNEY}: the Amiga's v6 cell");
    let model = b.session.screen();
    for honor in [true, false] {
        let host_state = tui_state(&b, honor);
        assert!(host_state.v6_page_pair.get().is_none(), "the host side must have no render-time cell");
        let host = for_model_compose(&model, &host_state);

        let tui_state = tui_state(&b, honor);
        tui_state.v6_page_pair.set(app::render::screen::v6_machine_pair(&model, honor));
        let (tui, tui_metrics) = tui_canvas(&model, &tui_state);

        eprintln!(
            "{AMIGA_JOURNEY} honor={honor}: canvas {}x{} · machine pair {:?} · host pair {:?} · metrics {tui_metrics:?}",
            tui.width(),
            tui.height(),
            app::render::screen::v6_machine_pair(&model, honor),
            app::render::screen::v6_host_pair(&tui_state),
        );
        let d = differing(&host.canvas, &tui);
        assert_eq!(d, 0, "{AMIGA_JOURNEY} honor={honor}: {d} pixels differ from the TUI's canvas");
        assert_eq!(host.metrics, tui_metrics, "{AMIGA_JOURNEY} honor={honor}: scroll metrics");
        assert!(tui_metrics.is_some(), "{AMIGA_JOURNEY} honor={honor}: no story box on this frame");

        let (stale, _) = tui_canvas(&model, &host_state);
        let stale_diff = differing(&stale, &tui);
        eprintln!("{AMIGA_JOURNEY} honor={honor}: the cell-less from_state path differs by {stale_diff} pixels");
        if honor {
            let (ink, page) = app::render::screen::v6_host_pair(&tui_state);
            assert_eq!(ink, image::Rgba([255, 255, 255, 255]), "the Amiga's ink is white (SQ-0740)");
            assert!(page[0] == page[1] && page[1] == page[2] && page[0] > 0 && page[0] < 255, "a grey page: {page:?}");
            assert_ne!((ink, page), app::render::screen::v6_host_pair(&host_state), "machine pair == host pair");
            assert!(stale_diff > 0, "the cell-less from_state path should have composed the host's pair");
        } else {
            assert_eq!(app::render::screen::v6_machine_pair(&model, honor), None);
            assert_eq!(stale_diff, 0, "declined: no machine pair, so the cell changes nothing");
        }
    }
}

/// `page` is the colour the canvas was flattened onto: with a host transcript that
/// draws nothing, it is every pixel of the story box no chrome glyph claimed.
#[test]
fn page_is_the_flatten_colour_inside_an_empty_story_box() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let f = host_compose_with(&b, honor, V6TextMode::RasteriseAndRecord, &empty_prose);
            let s = f.story.unwrap_or_else(|| panic!("{file} honor={honor}: no story box on this frame"));
            if !honor {
                assert_eq!(f.page, HOST_PAGE, "{file}: colours declined, so the page is the host's own");
            }
            assert!(f.caret.is_none(), "{file} honor={honor}: no live input, yet a caret was reported");
            // A chrome window the game printed INSIDE window 0 keeps its text there
            // (SQ-0728); those glyph boxes are the chrome's, not the page's.
            let claimed = |x: u32, y: u32| {
                f.text.iter().any(|r| {
                    (r.y..r.y + r.h).contains(&y) && r.boxes.iter().any(|&(bx, w)| (bx..bx + w).contains(&x))
                })
            };
            let mut checked = 0usize;
            for y in s.y..s.y + s.h {
                for x in s.x..s.x + s.w {
                    if claimed(x, y) {
                        continue;
                    }
                    checked += 1;
                    let p = *f.canvas.get_pixel(x, y);
                    assert_eq!(p, f.page, "{file} honor={honor}: ({x},{y}) inside the empty story box {s:?}");
                }
            }
            eprintln!("{file} honor={honor}: page {:?} over {checked} story-box pixels", f.page);
            assert!(checked > 0, "{file} honor={honor}: non-vacuity — the whole box was claimed");
        }
    }
}

/// **The caret is reported, and under `RecordOnly` no longer painted.** The canvas
/// with a live caret is the canvas without one, pixel for pixel, and the caret the
/// frame reports sits just after the last prose glyph. Under `Rasterise` it is still
/// drawn — exactly in the cell it reports.
#[test]
fn record_only_reports_the_caret_instead_of_painting_it() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let live = host_compose_with(&b, honor, V6TextMode::RecordOnly, &prose_awaiting(true));
            let idle = host_compose_with(&b, honor, V6TextMode::RecordOnly, &prose_awaiting(false));
            assert!(idle.caret.is_none(), "{file} honor={honor}: a caret with no live input");
            let c = live.caret.unwrap_or_else(|| panic!("{file} honor={honor}: awaiting input, no caret"));
            eprintln!("{file} honor={honor}: caret {c:?}");
            assert!(!c.panel, "{file} honor={honor}: the host's prose caret is not a panel's");
            assert!(live.canvas == idle.canvas, "{file} honor={honor}: RecordOnly painted the caret");
            // After the last prose glyph: the live input is empty, so the caret is
            // where the pen finished "What next?".
            let last = live
                .text
                .iter()
                .rev()
                .find(|r| r.source == v6::V6RunSource::StoryProse)
                .unwrap_or_else(|| panic!("{file} honor={honor}: no story prose among the runs"));
            assert_eq!(last.text, PROSE[1], "{file} honor={honor}: the last prose run");
            let (lx, _) = *last.boxes.last().expect("a run has a box");
            let end = lx + b.face.advance(last.text.chars().last().expect("non-empty"));
            assert_eq!((c.x, c.y, c.h), (end, last.y, last.h), "{file} honor={honor}: caret vs {last:?}");

            // Rasterise is unchanged: the caret is drawn, and ONLY the caret differs.
            let drawn = host_compose_with(&b, honor, V6TextMode::Rasterise, &prose_awaiting(true));
            let bare = host_compose_with(&b, honor, V6TextMode::Rasterise, &prose_awaiting(false));
            assert_eq!(drawn.caret, Some(c), "{file} honor={honor}: Rasterise reports the same caret");
            let cw = u32::from(b.face.cell().w());
            let mut differing = 0usize;
            for (x, y, p) in drawn.canvas.enumerate_pixels() {
                if bare.canvas.get_pixel(x, y) != p {
                    differing += 1;
                    assert!(
                        (c.x..c.x + cw).contains(&x) && (c.y..c.y + c.h).contains(&y),
                        "{file} honor={honor}: ({x},{y}) changed outside the caret cell {c:?}"
                    );
                }
            }
            assert!(differing > 0, "{file} honor={honor}: Rasterise no longer paints the caret");
        }
    }
}

// ── the caret's ink and width (SQ-1571) ──────────────────────────────────────

/// Shared assertion for SQ-1571: `c`, as `RecordOnly` reported it, must account
/// for exactly what `Rasterise` painted for the SAME frame — `drawn` is the live
/// frame with the caret rasterised, `idle` the identical frame with no caret
/// drawn at all, so every pixel the two disagree on is the caret's own paint and
/// nothing else. `c.ink` must be every one of those pixels' colour, and
/// `c.w * c.h` must equal their count — not merely contain them, as the looser
/// `record_only_reports_the_caret_instead_of_painting_it` check above does.
fn assert_caret_ink_and_footprint(label: &str, c: v6::V6Caret, drawn: &image::RgbaImage, idle: &image::RgbaImage) {
    assert_eq!(drawn.dimensions(), idle.dimensions(), "{label}: canvas sizes differ between the two frames");
    let mut painted = 0usize;
    for (x, y, p) in drawn.enumerate_pixels() {
        if idle.get_pixel(x, y) != p {
            painted += 1;
            assert!(
                (c.x..c.x + c.w).contains(&x) && (c.y..c.y + c.h).contains(&y),
                "{label}: ({x},{y}) changed outside the reported caret cell {c:?}"
            );
            assert_eq!(*p, c.ink, "{label}: ({x},{y}) inside the caret cell is not caret.ink");
        }
    }
    assert_eq!(
        painted,
        (c.w * c.h) as usize,
        "{label}: caret.w ({}) * caret.h ({}) does not equal the {painted} pixels Rasterise actually painted",
        c.w,
        c.h
    );
}

/// The ordinary story-window caret (Zork Zero r393): `RecordOnly`'s reported
/// `ink` and `w` match `Rasterise`'s actual paint exactly, for the same frame.
#[test]
fn record_only_story_caret_reports_the_rasterised_ink_and_width() {
    for (file, b) in specimens() {
        if file != "zork0-r393-s890714.z6" {
            continue;
        }
        for honor in [true, false] {
            let bare = host_compose_with(&b, honor, V6TextMode::RecordOnly, &prose_awaiting(true));
            let c = bare.caret.unwrap_or_else(|| panic!("{file} honor={honor}: awaiting input, no caret"));
            assert!(!c.panel, "{file} honor={honor}: the story caret reported as a panel's");
            let drawn = host_compose_with(&b, honor, V6TextMode::Rasterise, &prose_awaiting(true)).canvas;
            let idle = host_compose_with(&b, honor, V6TextMode::Rasterise, &prose_awaiting(false)).canvas;
            assert_caret_ink_and_footprint(&format!("{file} honor={honor}"), c, &drawn, &idle);
        }
    }
}

/// fmvpoker booted to its bet prompt — the same reach as
/// `v6_restore_input_window_echo.rs`'s `to_bet_prompt`: choosing "CHANGE CURRENT
/// BET" (SQ-0739) hands the read to the bottom panel, window 2, not window 0 —
/// the PANEL-caret half of SQ-1571's acceptance. `fmvpoker.z6` is one more
/// gitignored fixture the fetch script deliberately does not carry (see
/// `scripts/fixtures.manifest`'s "NOT FETCHED" note), so `fixture_path` falls
/// back to it only when the local `stories/` copy is there; this skips
/// vacuously like the specimens above without it.
fn fmvpoker_at_bet_prompt() -> Option<GameSession> {
    let path = crate::fixture_paths::fixture_path("fmvpoker.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(bytes, false, false, None, false, dims, picts.std_window(), None, None)
            .expect("fmvpoker (v6) boots");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let r = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    assert!(r.fault.is_none(), "fmvpoker faulted dismissing the title: {:?}", r.fault);
    let r = session.submit_char(b'c');
    assert!(r.fault.is_none(), "fmvpoker faulted choosing CHANGE CURRENT BET: {:?}", r.fault);
    assert_ne!(
        session.machine.screen.v6_input_window, 0,
        "premise: fmvpoker must be reading the bet through its bottom panel, not window 0"
    );
    Some(session)
}

/// Compose `session`'s current frame with no `Booted` `MachineBoot` chain behind
/// it — fmvpoker's own bare v6 default cell, matching `v6_fmvpoker_hybrid.rs`
/// and `v6_restore_input_window_echo.rs`'s own harnesses. `input: Some("")`
/// draws the panel's live caret with nothing typed yet (mirroring `host_prose`'s
/// empty `MainText::input` above); `None` suppresses it (SQ-1567 addendum).
///
/// `paint` is the game's own real ground (`erase_window` fills, SQ-0706), read
/// off `session` exactly as `v6_fmvpoker_hybrid.rs`'s `fmvpoker_title` does —
/// NOT `None`, or `fill_story_page_under_chrome_text` paints its flat `page`
/// straight over the panel's own felt background (and the caret drawn onto it),
/// since window 0's poker-table plate encloses rather than fills the screen and
/// the bet panel sits in its "clear" interior. That gap when the ground is
/// truly absent (a restore before the next `erase_window`) is
/// `v6_restore_input_window_echo.rs`'s own module doc, not this one.
fn fmvpoker_compose(session: &GameSession, text: V6TextMode, input: Option<&str>) -> app::render::screen::V6Frame {
    let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &tf);
    let layout = v6::classify_windows(items, tf.cell());
    let colors = app::colors::ColorScheme::terminal_default();
    let paint = Engine::paint_surface(session);
    let inputs = V6FrameInputs {
        host_pair: (HOST_INK, HOST_PAGE),
        honor_game_colours: false,
        colors: &colors,
        face: &tf,
        paint: paint.as_deref(),
        panel_input: input,
        input,
        prose: &empty_prose,
        reveal: None,
        pager_active: false,
        more_prompt_pair: (HOST_INK, HOST_PAGE),
        text,
        bottom_anchor_menu: false,
    };
    compose_v6_frame(&layout, RasterFrame::native(native), &inputs)
}

/// The PANEL caret (fmvpoker reading its bet prompt through window 2):
/// `RecordOnly`'s reported `ink` and `w` match `Rasterise`'s actual paint
/// exactly, for the same frame — the same acceptance as the story caret above,
/// on the OTHER call site `GlyphSink::caret` has (`draw_secondary_prose_into`).
#[test]
fn record_only_panel_caret_reports_the_rasterised_ink_and_width() {
    let Some(session) = fmvpoker_at_bet_prompt() else { return };
    let bare = fmvpoker_compose(&session, V6TextMode::RecordOnly, Some(""));
    let c = bare.caret.unwrap_or_else(|| panic!("fmvpoker: awaiting the bet, no caret"));
    eprintln!("fmvpoker (panel): caret {c:?}");
    assert!(c.panel, "fmvpoker: the panel caret reported as the story's");
    let drawn = fmvpoker_compose(&session, V6TextMode::Rasterise, Some("")).canvas;
    let idle = fmvpoker_compose(&session, V6TextMode::Rasterise, None).canvas;
    assert_caret_ink_and_footprint("fmvpoker (panel)", c, &drawn, &idle);
}

// ── the input line override (SQ-1567 addendum) ───────────────────────────────

/// **`input: None` suppresses the live input line entirely — text and caret
/// both — regardless of what the prose callback itself asked for.** A host that
/// owns its own input line can now keep its draft out of the composite without
/// ever touching `state.input.value`; this is the same idea `RecordOnly`
/// already applies to the caret's PAINTING, extended to the TEXT.
#[test]
fn a_host_input_override_of_none_suppresses_the_live_input_line() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            // Non-vacuity first: with the default (non-suppressing) override the
            // callback's `awaiting: true` really does draw a caret, or the case
            // below proves nothing about the override.
            let shown = host_compose_full(&b, honor, V6TextMode::Rasterise, &prose_awaiting(true), Some(""));
            assert!(shown.caret.is_some(), "{file} honor={honor}: non-vacuity — awaiting:true drew no caret at all");

            let idle = host_compose_full(&b, honor, V6TextMode::Rasterise, &prose_awaiting(false), Some(""));
            let suppressed = host_compose_full(&b, honor, V6TextMode::Rasterise, &prose_awaiting(true), None);
            assert!(suppressed.caret.is_none(), "{file} honor={honor}: input:None still reported a caret");
            assert_eq!(
                suppressed.canvas, idle.canvas,
                "{file} honor={honor}: input:None painted a live line anyway"
            );
        }
    }
}

/// **`input: Some(text)` draws exactly `text`, in place of whatever the prose
/// callback computed for the live line, with the caret one glyph past it.**
/// Lets a host preview its own draft in the game's own font without touching
/// `AppState`.
#[test]
fn a_host_input_override_draws_exactly_the_given_text() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let baseline =
                host_compose_full(&b, honor, V6TextMode::RasteriseAndRecord, &prose_awaiting(true), Some(""));
            let overridden =
                host_compose_full(&b, honor, V6TextMode::RasteriseAndRecord, &prose_awaiting(true), Some("Q"));
            let all = run_text(&overridden.text);
            assert!(all.contains('Q'), "{file} honor={honor}: the override text is not among the runs: {all}");
            let base_caret =
                baseline.caret.unwrap_or_else(|| panic!("{file} honor={honor}: no caret with an empty draft"));
            let over_caret =
                overridden.caret.unwrap_or_else(|| panic!("{file} honor={honor}: no caret with a draft"));
            assert_eq!(
                (over_caret.y, over_caret.h, over_caret.panel),
                (base_caret.y, base_caret.h, base_caret.panel),
                "{file} honor={honor}: the caret stays on the input row"
            );
            let adv = b.face.advance('Q');
            assert_eq!(
                over_caret.x,
                base_caret.x + adv,
                "{file} honor={honor}: the caret sits one glyph past the override text"
            );
            assert_ne!(overridden.canvas, baseline.canvas, "{file} honor={honor}: the override text painted no pixel");
        }
    }
}

/// Every run says which part of the composite drew it. Zork Zero's status runs are
/// chrome and the host's prose is story prose; Journey's menu is not story prose.
#[test]
fn every_run_names_its_source() {
    use v6::V6RunSource as Src;
    for (file, b) in specimens() {
        for honor in [true, false] {
            let f = host_compose(&b, honor, V6TextMode::RecordOnly);
            let summary: Vec<String> =
                f.text.iter().map(|r| format!("{:?}@{}:{:?}", r.source, r.y, r.text.trim())).collect();
            eprintln!("{file} honor={honor}: {}", summary.join(" | "));
            for r in &f.text {
                let host = PROSE.iter().any(|p| p.contains(r.text.as_str()));
                if r.source == Src::StoryProse {
                    assert!(host, "{file} honor={honor}: {r:?} is story prose the host never wrote");
                }
            }
            let prose: Vec<&str> =
                f.text.iter().filter(|r| r.source == Src::StoryProse).map(|r| r.text.as_str()).collect();
            assert_eq!(prose, PROSE, "{file} honor={honor}: the host's prose, and only it, is StoryProse");
            let game: Vec<&V6TextRun> = f.text.iter().filter(|r| r.source != Src::StoryProse).collect();
            assert!(!game.is_empty(), "{file} honor={honor}: non-vacuity — no chrome run on this frame");
            match file {
                "zork0-r393-s890714.z6" => {
                    for r in &game {
                        assert!(
                            matches!(r.source, Src::Chrome | Src::GridCell),
                            "{file} honor={honor}: a status run that is not chrome: {r:?}"
                        );
                    }
                }
                "journey-r83-s890706.z6" => {
                    // The menu strip under the text panel is a grid window's
                    // PIXEL-positioned runs, so every run there is `Chrome` — measured,
                    // not assumed: the verbs and the party's column headings.
                    let s = f.story.expect("Journey r83 has a story box on this frame");
                    let menu: Vec<&&V6TextRun> = game.iter().filter(|r| r.y >= s.y + s.h).collect();
                    for label in ["The Party", "Individual Commands", "Start", "Background", "Help"] {
                        assert!(
                            menu.iter().any(|r| r.text == label),
                            "{file} honor={honor}: non-vacuity — menu label {label:?} not below the story box"
                        );
                    }
                    for r in &menu {
                        assert_eq!(r.source, Src::Chrome, "{file} honor={honor}: a menu run: {r:?}");
                    }
                }
                other => panic!("no source expectation pinned for {other}"),
            }
        }
    }
}

/// The anchor for the TUI side above: on this frame `render_story_pane` publishes
/// exactly the pair [`v6_machine_pair`](app::render::screen::v6_machine_pair)
/// derives from the model, in both honour modes.
#[test]
fn the_terminal_render_publishes_the_pair_the_model_states() {
    let Some(b) = amiga_journey_after_restore() else { return };
    let model = b.session.screen();
    for honor in [true, false] {
        let mut state = tui_state(&b, honor);
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let pane = ratatui::layout::Rect::new(0, 0, 100, 40);
        let mut buf = ratatui::buffer::Buffer::empty(pane);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, pane, &mut buf);
        let derived = app::render::screen::v6_machine_pair(&model, honor);
        assert_eq!(state.v6_page_pair.get(), derived, "honor={honor}");
        assert_eq!(derived.is_some(), honor, "honor={honor}: the Amiga frame has a machine pair only while honoured");
    }
}

/// Zork Zero r393 and Journey r83 have no machine pair, so `for_model` composes
/// them exactly as the TUI always has.
#[test]
fn the_ordinary_presses_compose_unchanged_from_the_model() {
    for (file, b) in specimens() {
        let model = b.session.screen();
        for honor in [true, false] {
            assert_eq!(app::render::screen::v6_machine_pair(&model, honor), None, "{file} honor={honor}");
            let state = tui_state(&b, honor);
            let host = for_model_compose(&model, &state);
            let (tui, tui_metrics) = tui_canvas(&model, &state);
            let d = differing(&host.canvas, &tui);
            assert_eq!(d, 0, "{file} honor={honor}: {d} pixels differ from the TUI's canvas");
            assert_eq!(host.metrics, tui_metrics, "{file} honor={honor}");
        }
    }
}

// ── V6Frame::ink — the story ink the composite paints with (SQ-1573) ────────
//
// A host drawing v6 prose itself under `RecordOnly` used to have to restate
// `compose_v6_frame_into`'s own ink rule — `v6::story_fg_rgba` over the story
// window, falling back to the host pair's ink — to match what `Rasterise` would
// have painted. `V6Frame::ink` is now that one resolved value, read off the
// frame instead of re-derived.

/// Every story-prose glyph pixel `RasteriseAndRecord` painted and `RecordOnly`
/// left unpainted is `frame.ink`, and nothing else — not "some colour", the
/// EXACT one `Rasterise` uses.
fn assert_ink_matches_every_prose_pixel(label: &str, full: &app::render::screen::V6Frame, bare: &app::render::screen::V6Frame) {
    assert_eq!(full.ink, bare.ink, "{label}: ink must not depend on the text mode");
    let mut painted = 0usize;
    for (x, y, p) in full.canvas.enumerate_pixels() {
        if bare.canvas.get_pixel(x, y) == p {
            continue;
        }
        let inside_prose = full.text.iter().any(|r| {
            r.source == v6::V6RunSource::StoryProse && (r.y..r.y + r.h).contains(&y) && r.boxes.iter().any(|&(bx, w)| (bx..bx + w).contains(&x))
        });
        if inside_prose {
            painted += 1;
            assert_eq!(*p, full.ink, "{label}: ({x},{y}) is a story-prose glyph pixel but not frame.ink");
        }
    }
    assert!(painted > 0, "{label}: non-vacuity — no story-prose glyph pixel differed");
}

/// **The acceptance case, Zork Zero r393 and Journey r83.**
#[test]
fn frame_ink_is_the_colour_rasterise_paints_the_prose_in() {
    for (file, b) in specimens() {
        for honor in [true, false] {
            let full = host_compose(&b, honor, V6TextMode::RasteriseAndRecord);
            let bare = host_compose(&b, honor, V6TextMode::RecordOnly);
            assert_ink_matches_every_prose_pixel(&format!("{file} honor={honor}"), &full, &bare);
        }
    }
}

/// [`for_model_compose`], with the text mode named by the caller — `for_model`
/// always builds [`V6TextMode::Rasterise`] inputs, so this overrides the field
/// afterward (public on [`V6FrameInputs`]) to reach `RasteriseAndRecord`/`RecordOnly`.
fn for_model_compose_with_text(
    model: &app::engine::ScreenModel,
    state: &app::state::AppState,
    text: V6TextMode,
) -> app::render::screen::V6Frame {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let paint = state.v6_paint.borrow();
    let prose = |cols: u16, rows: u16| app::render::screen::build_main_text(state, cols, rows);
    let mut inputs = V6FrameInputs::for_model(state, model, paint.as_deref(), &prose, Some(state.input.value.as_str()));
    inputs.text = text;
    compose_v6_frame(&layout, RasterFrame::native(native), &inputs)
}

/// **The acceptance case, the Amiga Journey floppy** — its own machine page pair,
/// composed from the model alone (SQ-1566's own path). `frame.ink` still matches
/// `Rasterise`'s paint exactly under this press's pair, in both honour modes.
#[test]
fn frame_ink_matches_rasterise_on_the_amiga_journey_floppy() {
    let Some(b) = amiga_journey_after_restore() else { return };
    let model = b.session.screen();
    for honor in [true, false] {
        let state = tui_state(&b, honor);
        let full = for_model_compose_with_text(&model, &state, V6TextMode::RasteriseAndRecord);
        let bare = for_model_compose_with_text(&model, &state, V6TextMode::RecordOnly);
        assert_ink_matches_every_prose_pixel(&format!("{AMIGA_JOURNEY} honor={honor}"), &full, &bare);
    }
}

// ── InlineImage::margin_px — the gutter beside a v6 picture (SQ-1573) ────────
//
// `InlineImage::margin_px` was already `pub`, all the way from where a window-0
// float is constructed (`session.rs`) through `TranscriptElem::Image` to
// `AppState::transcript_images` — this is the test that was missing, proving the
// raw value a host reads there is the SAME one the raster layout actually used
// to place text beside the picture, not merely present.
//
// The raster wrap's own rule (`render/wrap_cache.rs::raster_wrap_extend`, private
// to the crate) is simple once the picture is already in the v6 unit-pixel space
// the text cell is stated in (SQ-0479): `margin_px` (or, absent one, the
// picture's own width plus one cell) divided up by the cell width, rounded up.
// This mirrors that formula — a host outside the crate cannot call the private
// fn, so this is the same computation a host would have to make from the public
// field.
fn expected_float_text_col(margin_px: Option<u32>, native_w: u32, cell_w: u16) -> u16 {
    let cell_w = u32::from(cell_w.max(1));
    margin_px.unwrap_or(native_w + cell_w).div_ceil(cell_w) as u16
}

/// Boot Zork Zero r393 fresh and accumulate the boot banner through the real
/// elems pipeline (`Engine::take_transcript_elems`), so `state.transcript_images`
/// carries the window-0 floats (the ornate drop-cap and the room icon) exactly as
/// a live session would — the same boot `v6_float_machine_page::frame` uses. This
/// is a second boot path (rather than reusing [`boot_at`]) because `boot_at`
/// already drains the plain transcript with `session.take_transcript()`, which
/// would starve `take_transcript_elems`'s own sink drain of the very prose the
/// float sizes itself against.
fn zork0_real_transcript(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let path = stories_dir().join("zork0-r393-s890714.z6");
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
    let honoured = honor && !picts.declines_game_colours(profile.default_colours());
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
    let mut session = GameSession::new_for_machine(bytes, honoured, false, false, dims, None, None, &boot)
        .unwrap_or_else(|e| panic!("zork0-r393: should boot without a ZError: {e:?}"));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(profile.palette());
    state.config.v6_render = app::config::V6RenderMode::Raster;
    state.config.honor_game_colours = honoured;
    state.v6_art_scale = art_scale.unwrap_or((2, 2));
    state.v6_text = face;
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);
    Some((session, state))
}

/// **The acceptance case.** `InlineImage::margin_px`, read straight off the
/// drop-cap `AppState::transcript_images` carries, reconstructs the raster
/// layout's own text column exactly — and the pixel `RecordOnly` reports for the
/// first prose glyph beside the picture lands exactly there too, not merely at
/// some nonzero offset.
#[test]
fn drop_cap_margin_px_matches_where_the_raster_path_places_the_text_pixel() {
    for honor in [true, false] {
        let Some((session, state)) = zork0_real_transcript(honor) else { return };
        let img = state
            .transcript_images
            .iter()
            .flatten()
            .find(|i| i.margin_px.is_some())
            .unwrap_or_else(|| panic!("honor={honor}: no margin-carrying float on Zork Zero's boot banner"));
        let cell = state.v6_text.cell();
        eprintln!(
            "honor={honor}: drop-cap margin_px={:?} native_w={} cell_w={}",
            img.margin_px,
            img.pixels.width(),
            cell.w()
        );

        let (main, _) = app::render::screen::build_main_text(&state, 70, 30);
        let rf = main
            .floats
            .iter()
            .find(|f| std::sync::Arc::ptr_eq(&f.img, &img.pixels))
            .unwrap_or_else(|| panic!("honor={honor}: the margin-carrying picture has no float in the raster layout"));
        let expected_col = expected_float_text_col(img.margin_px, img.pixels.width(), cell.w());
        assert_eq!(
            rf.text_col, expected_col,
            "honor={honor}: margin_px does not reconstruct the raster layout's own text column"
        );

        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let native = v6::native_extent(items, &state.v6_text);
        let layout = v6::classify_windows(items, state.v6_text.cell());
        let paint = state.v6_paint.borrow();
        let prose = |c: u16, r: u16| app::render::screen::build_main_text(&state, c, r);
        let inputs = V6FrameInputs {
            host_pair: (HOST_INK, HOST_PAGE),
            honor_game_colours: honor,
            colors: &state.colors,
            face: &state.v6_text,
            paint: paint.as_deref(),
            panel_input: None,
            input: None,
            prose: &prose,
            reveal: None,
            pager_active: false,
            more_prompt_pair: (HOST_INK, HOST_PAGE),
            text: V6TextMode::RecordOnly,
            bottom_anchor_menu: false,
        };
        let f = compose_v6_frame(&layout, RasterFrame::native(native), &inputs);
        let s = f.story.unwrap_or_else(|| panic!("honor={honor}: no story box on Zork Zero's boot frame"));
        let row_top = s.y + (rf.row.max(0) as u32) * u32::from(cell.h());
        let painted = f
            .text
            .iter()
            .find(|r| r.source == v6::V6RunSource::StoryProse && r.y == row_top)
            .unwrap_or_else(|| panic!("honor={honor}: no story prose recorded on the float's own row {row_top}"));
        let (px, _) = *painted.boxes.first().expect("a run has a box");
        let expected_x = s.x + u32::from(rf.text_col) * u32::from(cell.w());
        assert_eq!(px, expected_x, "honor={honor}: the prose beside the drop-cap is not at margin_px's own column");
    }
}

// ---------------------------------------------------------------------------
// SQ-1574: `hybrid_bottom_plan_for` — the private Hybrid `BottomPlan`,
// published for a host that draws its own v6 chrome and so has no path to the
// TUI's `build_hybrid_frame` to read it off.
//
// Each case renders the SAME frame through the TUI's real Hybrid path (a real
// `render_story_pane` call, at a real 8x18-ish kitty cell) and reads the plan
// it actually took off `state.v6_ring_plan` — the same oracle
// `v6_journey_menu_band.rs` pins the `Menu` plan against — then asks
// `hybrid_bottom_plan_for` the identical question and compares. `slack_native_
// rows` only ever gates the `Letterbox` branch (zero vs. nonzero), so a case
// passes whichever of `0`/`1` matches the pane it rendered the oracle at,
// never a value derived by re-implementing the render's own slack arithmetic —
// which would just restate the thing under test.
// ---------------------------------------------------------------------------

/// A pane tall enough to leave real vertical slack below the story window at
/// any of this suite's titles — [`v6_extended_frame`]'s own `TALL`, reused
/// because it is already proven to reach `Extend`/`Frame`/`Menu` on this exact
/// corpus (module doc there). CELL matches `v6_journey_menu_band.rs`'s own
/// sweep (8x18).
const PLAN_TALL: (u16, u16) = (100, 50);
/// A pane wide enough that the vertical axis is always the binding one — the
/// letterbox margin lands left/right instead of top/bottom, so the SLACK this
/// suite's `hybrid_bottom_plan` asks about is zero by construction, whatever
/// the title. Confirmed against the real oracle in
/// `hybrid_bottom_plan_for_matches_the_tuis_own_decision` rather than merely
/// asserted here.
const PLAN_SNUG: (u16, u16) = (240, 23);
const PLAN_CELL: (u16, u16) = (8, 18);

/// Render `b`'s current frame through the TUI's real Hybrid path at `pane`, and
/// read back the plan it took (`state.v6_ring_plan`) alongside
/// `hybrid_bottom_plan_for`'s own answer for the identical layout/native/cell —
/// `slack_native_rows` is `0` at [`PLAN_SNUG`], `1` (any nonzero placeholder,
/// per the function's own doc) at [`PLAN_TALL`].
#[allow(deprecated)]
fn plan_oracle_and_answer(b: &Booted, honor: bool, pane: (u16, u16)) -> (&'static str, V6BottomPlan) {
    let mut state = tui_state(b, honor);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(PLAN_CELL.0, PLAN_CELL.1)));
    let model = b.session.screen();
    let area = ratatui::layout::Rect::new(0, 0, pane.0, pane.1);
    let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let oracle = state.v6_ring_plan.get();

    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let slack = if pane == PLAN_SNUG { 0 } else { 1 };
    let answer = app::render::screen::hybrid_bottom_plan_for(&layout, native, state.v6_text.cell(), slack).kind;
    (oracle, answer)
}

fn assert_plan(file: &str, honor: bool, pane: (u16, u16), oracle: &str, answer: V6BottomPlan, want: V6BottomPlan) {
    assert_eq!(
        oracle, want_str(want),
        "{file} honor={honor} {pane:?}: premise — the TUI's own Hybrid render must take the {want:?} plan \
         for this case to test what it says it does. Got {oracle:?}"
    );
    assert_eq!(
        answer, want,
        "{file} honor={honor} {pane:?}: hybrid_bottom_plan_for disagreed with the TUI's own Hybrid render \
         (oracle {oracle:?})"
    );
}

/// Arthur one tap past the mount (the title CARD, before the poles have grown
/// to enclose the story window) — the one frame in his corpus where they stop
/// short of the screen bottom on both sides (`Extend`), measured by sweeping
/// taps against the real oracle: `Extend` through tap 4, `Frame` from tap 5
/// onward through `look`. [`boot`]'s generic space/taps=6 (or
/// `v6_extended_frame.rs`'s own 12-tap-then-`look` specimen) lands past that
/// point, on the `Frame` frame instead — a different, equally real Arthur
/// frame, just not this suite's `Extend` case.
fn boot_arthur_extend() -> Option<Booted> {
    let mut b = boot_at("arthur-r74-s890714.z6", 74)?;
    match b.session.pending_input() {
        InputKind::Line | InputKind::Event => {
            b.session.submit("");
        }
        InputKind::Char => {
            b.session.submit_char(b'n');
        }
    }
    Some(b)
}

fn want_str(p: V6BottomPlan) -> &'static str {
    match p {
        V6BottomPlan::Letterbox => "letterbox",
        V6BottomPlan::Extend => "extend",
        V6BottomPlan::Frame => "frame",
        V6BottomPlan::Menu => "menu",
    }
}

/// Menu (Journey, both releases — a disk image is a different BUILD, CLAUDE.md),
/// Extend (Arthur), Frame (Zork Zero, Shogun) at [`PLAN_TALL`], and Letterbox at
/// [`PLAN_SNUG`] on the same corpus — `hybrid_bottom_plan_for` takes the answer
/// the TUI's own private `hybrid_bottom_plan` actually reaches, on every shape it
/// has, not a hardcoded expectation.
#[test]
fn hybrid_bottom_plan_for_matches_the_tuis_own_decision() {
    let cases: &[(&str, u16, V6BottomPlan)] = &[
        ("journey-r83-s890706.z6", 83, V6BottomPlan::Menu),
        (AMIGA_JOURNEY, 30, V6BottomPlan::Menu),
        ("zork0-r393-s890714.z6", 393, V6BottomPlan::Frame),
        ("shogun-r322-s890706.z6", 322, V6BottomPlan::Frame),
    ];
    for (file, release, want) in cases.iter().copied() {
        let Some(b) = boot(file, release) else { continue };
        for honor in [true, false] {
            let (oracle, answer) = plan_oracle_and_answer(&b, honor, PLAN_TALL);
            assert_plan(file, honor, PLAN_TALL, oracle, answer, want);
        }
    }
    let Some(b) = boot_arthur_extend() else { return };
    for honor in [true, false] {
        let (oracle, answer) = plan_oracle_and_answer(&b, honor, PLAN_TALL);
        assert_plan("arthur-r74-s890714.z6", honor, PLAN_TALL, oracle, answer, V6BottomPlan::Extend);
    }
}

/// A pane whose vertical axis is always the binding one — no slack to reclaim,
/// whatever the title — takes `Letterbox`. Journey is not a specimen here: its
/// `Menu` plan is decided BEFORE slack is (SQ-0830, `hybrid_bottom_plan`'s own
/// doc) — a command menu is a fact about the frame, not the pane — so it never
/// takes `Letterbox` at any pane and is covered by the `Menu` case above instead.
#[test]
fn hybrid_bottom_plan_for_is_letterbox_with_no_slack() {
    let cases: &[(&str, u16)] = &[
        ("zork0-r393-s890714.z6", 393),
        ("shogun-r322-s890706.z6", 322),
        ("arthur-r74-s890714.z6", 74),
    ];
    for (file, release) in cases.iter().copied() {
        let Some(b) = boot(file, release) else { continue };
        for honor in [true, false] {
            let (oracle, answer) = plan_oracle_and_answer(&b, honor, PLAN_SNUG);
            assert_plan(file, honor, PLAN_SNUG, oracle, answer, V6BottomPlan::Letterbox);
        }
    }
}
