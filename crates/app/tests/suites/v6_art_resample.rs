//! SQ-0824: art smaller than its own artwork is resampled ONCE, through a filter
//! chosen by the direction it moves in.
//!
//! The report, on `Journey - The Quest Begins.adf` (**release 30, serial 890322** —
//! the Amiga floppy, a different build from `journey-r83-s890706.z6`): distortion in
//! the canyon plate *only when the artwork is smaller*, worst in the finest detail —
//! the foreground rocks and the dithered shadow — read as "maybe scaled up then
//! scaled down". Measurement off the user's own screenshots had already ruled out
//! tiling and combing, and found nothing region-specific: the lower quarter is simply
//! where the finest detail lives.
//!
//! What the pane sweep found. Journey's plate is 222x254 native pixels and reaches the
//! screen through the HYBRID ring's stretched-band path, whose device box tracks the
//! pane: 168x198 at a 60x24 pane, 200x234 at 70x30, 328x378 at 117x64. Below roughly
//! 80 columns (at an 8px cell) both axes SHRINK — and every one of the three art paths
//! shrank with `FilterType::Nearest`, which drops whole source rows and columns: 54 of
//! 222 columns and 56 of 254 rows never sampled at all at the smallest pane. On a
//! dithered plate "some pixels survive and their neighbours don't" is exactly the
//! aliasing that was reported, and it is confined to the shrinking regime, which is
//! why the artwork only looked wrong when it was small.
//!
//! The raster composite had a second, milder version of the same thing: its own
//! pre-scale was clamped at 1.0, so a pane smaller than the composite made a full
//! identity copy that bought nothing and then left the actual shrink to the protocol's
//! default Nearest fit.
//!
//! This suite pins the REGIME — that the sweep really does put the plate on both sides
//! of 1:1, and that the band log names the filter each band went through, which no cell
//! rect ever could. The resampler's quality is measured against a per-axis
//! single-resample ideal in `render::graphics::resample_tests`.
//!
//! FALSIFY by restoring `FilterType::Nearest` in `resize_directional`: the direction
//! cases below still pass (they pin geometry, not pixels) while the unit cases fail —
//! which is the division of labour intended, since the gitignored floppy cannot gate
//! CI. Both `honor_game_colours` modes, per the project's colour-render convention.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The Amiga floppy, and the build it must be.
const FIXTURE: &str = "Journey - The Quest Begins.adf";
const RELEASE: u16 = 30;
const SERIAL: &str = "890322";

/// A plausible kitty font cell. `Picker::halfblocks()` reports 1x2, a regime in which
/// no scale defect reproduces at all (the SQ-0548 lesson).
const CELL: (u16, u16) = (8, 18);

/// Panes either side of the regime boundary. The first four put Journey's 222x254 plate
/// below its native size on both axes — where the report lives — and the last three
/// above it.
const PANES: [(u16, u16); 7] =
    [(60, 24), (70, 30), (76, 28), (78, 34), (100, 40), (117, 64), (160, 60)];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the floppy exactly as `startup.rs` does — the profile comes from the medium —
/// and drive the intro to the Praxix command menu, the frame that carries the plate.
/// `None` (with a SKIP note) when the gitignored medium is absent.
fn journey_floppy_at_menu() -> Option<GameSession> {
    let path = stories_dir().join(FIXTURE);
    let (loaded, mounted) = match app::hints::load_mounted_story(&path) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    assert!(mounted.is_some(), "{FIXTURE}: the mount must report a floppy, not a bare story file");
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        RELEASE,
        "{FIXTURE}: this medium carries a DIFFERENT build than release {RELEASE}"
    );
    assert_eq!(&String::from_utf8_lossy(&bytes[0x12..0x18]), SERIAL, "{FIXTURE}: serial");

    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey release 30 boots without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    assert!(!session.quit && session.machine.fault_trace.is_none(), "booted clean");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

#[allow(deprecated)]
fn render(
    model: &app::engine::ScreenModel,
    mode: app::config::V6RenderMode,
    honor: bool,
    cols: u16,
    rows: u16,
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker = app::render::graphics::kitty_picker(CELL.0, CELL.1);
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    state
}

/// Every `resample W×H->W×H x:… y:…` clause this frame's band log carries, parsed back
/// into `(src_w, src_h, dst_w, dst_h, x_filter, y_filter)`.
fn resamples(state: &app::state::AppState) -> Vec<(u32, u32, u32, u32, String, String)> {
    state
        .graphics_render
        .borrow()
        .band_log
        .iter()
        .filter_map(|line| {
            let tail = line.split("resample ").nth(1)?;
            let mut parts = tail.split_whitespace();
            let dims = parts.next()?;
            let (src, dst) = dims.split_once("->")?;
            let (sw, sh) = src.split_once('x')?;
            let (dw, dh) = dst.split_once('x')?;
            let fx = parts.next()?.strip_prefix("x:")?.to_string();
            let fy = parts.next()?.strip_prefix("y:")?.to_string();
            Some((sw.parse().ok()?, sh.parse().ok()?, dw.parse().ok()?, dh.parse().ok()?, fx, fy))
        })
        .collect()
}

/// (a) Every band the hybrid ring draws names its resample, and the filter it names is
/// the one its own direction calls for. This is the whole rule, stated where it is
/// observable on the real artwork rather than on a synthetic plate.
#[test]
fn every_hybrid_band_resamples_in_the_direction_it_moves() {
    let Some(session) = journey_floppy_at_menu() else { return };
    let model = session.screen();
    let mut seen = 0usize;
    for honor in [true, false] {
        for (cols, rows) in PANES {
            let state = render(&model, app::config::V6RenderMode::Hybrid, honor, cols, rows);
            let rs = resamples(&state);
            assert!(
                !rs.is_empty(),
                "{FIXTURE} r{RELEASE} honor={honor} pane {cols}x{rows}: the hybrid ring drew no \
                 band that resamples — the sweep is not exercising the art path at all"
            );
            for (sw, sh, dw, dh, fx, fy) in rs {
                seen += 1;
                let want = |d: u32, s: u32| if d < s { "area" } else { "nearest" };
                assert_eq!(
                    (fx.as_str(), fy.as_str()),
                    (want(dw, sw), want(dh, sh)),
                    "{FIXTURE} r{RELEASE} honor={honor} pane {cols}x{rows}: {sw}x{sh}->{dw}x{dh} \
                     went through x:{fx} y:{fy}. Nearest on a SHRINKING axis drops whole source \
                     rows and columns, which is the reported aliasing in the dithered detail; a \
                     smoothing filter on a GROWING one is the crispness this must not cost."
                );
            }
        }
    }
    assert!(seen >= 14, "the sweep measured only {seen} band resamples across 14 renders");
}

/// (b) The sweep straddles 1:1 — the plate really is minified at the small panes and
/// magnified at the large ones. Without this the case above passes vacuously on a sweep
/// that never leaves the magnifying regime, which is the regime that was always fine.
#[test]
fn the_sweep_puts_the_plate_on_both_sides_of_its_native_size() {
    let Some(session) = journey_floppy_at_menu() else { return };
    let model = session.screen();
    let (mut shrank, mut grew) = (Vec::new(), Vec::new());
    for (cols, rows) in PANES {
        let state = render(&model, app::config::V6RenderMode::Hybrid, true, cols, rows);
        // The plate is the largest band the ring resamples; the thin frame rules are
        // the small ones.
        let Some(&(sw, sh, dw, dh, _, _)) =
            resamples(&state).iter().max_by_key(|(sw, sh, ..)| sw * sh)
        else {
            panic!("{FIXTURE} r{RELEASE} pane {cols}x{rows}: no band resampled")
        };
        assert!(
            sw >= 200 && sh >= 200,
            "{FIXTURE} r{RELEASE} pane {cols}x{rows}: the largest resampled band is {sw}x{sh} \
             native — the 222x254 canyon plate is not being drawn, so this sweep measures \
             something else"
        );
        if dw < sw && dh < sh {
            shrank.push((cols, rows, dw, dh));
        } else if dw > sw && dh > sh {
            grew.push((cols, rows, dw, dh));
        }
    }
    assert!(
        shrank.len() >= 3,
        "only {} of the swept panes minify the plate ({shrank:?}) — the defect's own regime \
         has to be covered by more than one size, because 'only when the artwork is smaller' \
         is the whole shape of this report",
        shrank.len()
    );
    assert!(!grew.is_empty(), "no swept pane magnifies the plate ({grew:?})");
}

/// (c) The raster composite is in the same two regimes, and its own pre-scale no longer
/// stands between the canvas and the shrink. The click map records the extent the
/// composite was actually drawn at, which is the net scale in the only unit that path
/// exposes.
#[test]
fn the_raster_composite_shrinks_below_its_canvas_at_a_small_pane() {
    let Some(session) = journey_floppy_at_menu() else { return };
    let model = session.screen();
    let (mut shrank, mut grew) = (0usize, 0usize);
    for honor in [true, false] {
        for (cols, rows) in PANES {
            let state = render(&model, app::config::V6RenderMode::Raster, honor, cols, rows);
            let map = state
                .graphics_render
                .borrow()
                .last_v6_map
                .clone()
                .unwrap_or_else(|| panic!("pane {cols}x{rows}: the raster path drew no composite"));
            assert_eq!(
                (map.canvas, map.screen),
                ((640, 400), (640, 400)),
                "{FIXTURE} r{RELEASE} honor={honor} pane {cols}x{rows}: the composite's native \
                 canvas is Journey's 320x200 screen at the uniform V6_ART_SCALE"
            );
            if map.img_w < map.canvas.0 as f32 {
                shrank += 1;
            } else if map.img_w > map.canvas.0 as f32 {
                grew += 1;
            }
        }
    }
    assert!(
        shrank >= 6 && grew >= 4,
        "{FIXTURE} r{RELEASE}: {shrank} of 14 raster renders shrink the composite and {grew} \
         magnify it — the sweep must cover both, since only the shrinking one was ever wrong"
    );
}
