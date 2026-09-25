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
    host_compose_with(b, honor, text, &host_prose)
}

/// [`host_compose`] with the host's prose callback named by the caller.
fn host_compose_with(
    b: &Booted,
    honor: bool,
    text: V6TextMode,
    prose: &dyn Fn(u16, u16) -> (MainText, RasterMetrics),
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
        panel_input: Some(""),
        prose,
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
