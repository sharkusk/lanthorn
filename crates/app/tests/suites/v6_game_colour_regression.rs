//! SQ-0532 wave-3: reproducers for the three v6 render regressions the
//! compliance waves introduced, found by driving the real games with the app's
//! REAL config defaults.
//!
//! # Why the existing v6 smokes all missed these
//!
//! `Config::honor_game_colours` defaults to **true**, but every pre-existing v6
//! test boots with `honor_game_colours = false`. With colours declined the games
//! never call `set_colour`, so the two wave-1 changes below have nothing to
//! propagate and the render looks fine. Turn the flag on — as every real player
//! run does — and Zork Zero's pane goes solid white. The tests here therefore
//! boot with `colours = true`, and the Shogun case renders the SPLASH (which no
//! existing test does) rather than gameplay.
//!
//! # The three failures, and their mechanisms
//!
//! **1 + 2 — Zork Zero is entirely white in raster and hybrid, with no art.**
//! Two independent wave-1 deltas stack:
//!
//! - `erase_window` now blanks a window's character grid with `clear_to(bg)`
//!   instead of `clear()`, stamping an EXPLICIT background colour into every
//!   cell. Zork0 boots with `set_colour(fg=2, bg=9, window=0)`, then
//!   `set_colour(..., window=7)`, then `erase_window(-1)`. §8.8.5.3.1 says -1
//!   erases to window 0's background, so all 2000 cells of window 7 — the
//!   FULL-SCREEN decorative window — become explicitly white. The screen model
//!   paints graphics first and text windows on top, so that stale opaque white
//!   grid covers every picture for the rest of the session. (The erase is
//!   already expressed to the pixel path as a canvas-clear event; stamping the
//!   text grid too applies it twice, on the layer that wins.)
//! - `mirror_v6_colours` now copies the current window's pair into
//!   `screen.current_fg/current_bg`, which `v6_screen_model` publishes as
//!   `ScreenModel.bg/fg` — the pane's page colour. It used to stay
//!   `ZColour::Default` for v6 (nothing wrote it), letting the host theme supply
//!   the page; it is now an explicit white, which floods the raster page.
//!
//! Measured on `zork0-r393-s890714.z6` at 80×30, counting cell backgrounds:
//!
//! | model under test                        | hybrid white / art | raster white |
//! |-----------------------------------------|--------------------|--------------|
//! | as shipped                              | 2400 / 0           | 2400         |
//! | with `ScreenModel.bg` neutralised       |  950 / 0           |    0         |
//! | with the grid cell bgs neutralised      | 1450 / 950         | 2400         |
//! | with both neutralised (≈ pre-wave)      |    0 / 950         |    0         |
//!
//! so BOTH deltas must be corrected — neither alone restores the picture.
//!
//! **3 — Shogun's splash is blank in hybrid** (raster still shows it). Wave 1
//! made v6 window 0 boot at the full screen size (§8.8.3.3, "Window 0 occupies
//! the whole screen") instead of height 0. `v6_screen_model` skips zero-size
//! windows, so before the game splits, window 0 used to be absent, and
//! `classify_windows` reported `story: None` — which is exactly how hybrid mode
//! decides to fall through to the full-art raster path. Now window 0 is present
//! at 640×400, hybrid takes the story-viewport path, and the terminal transcript
//! viewport covers the whole pane with the splash art reduced to a chrome ring
//! of zero thickness. This one reproduces with colours off too.
//!
//! # Status — FIXED (wave 4), these now guard the fixes
//!
//! 1. The v6 `erase_window` arms went back to `grid.clear()`: a v6 character
//!    grid is a compositing layer drawn OVER the picture canvases, so a blank
//!    cell must stay `ZColour::Default`/transparent. §8.8.5.3's erase colour
//!    still reaches the host — as each window's own `bg` and as the number-0
//!    canvas-clear event the same arm already queues. (The v1–5 upper window
//!    keeps `clear_to(bg)`: it has no art beneath it, so an explicit background
//!    there is exactly §8.7.3.2's "cleared to background colour".)
//! 2. `v6_screen_model` no longer publishes `screen.current_fg/bg` as
//!    `ScreenModel.bg/fg`. That field is the PANE PAGE, and a v6 story has none
//!    — every window carries its own pair (§8.3) and publishes it on its own
//!    node. The §8.3 mirror itself stays: v6 prose runs still get the current
//!    window's pair in their `TextAttrs`.
//! 3. `v6_screen_model` skips window 0 while it is still the untouched
//!    whole-screen boot window (§8.8.3.3) with nothing in it — no painted runs,
//!    no streamed characters, no canvas. `get_wind_prop` keeps reporting the
//!    real 640×400 dimensions (the wave-1 spec fix); only the RENDER declines to
//!    treat an empty placeholder as the story page.
//!
//! Both colour modes are covered: the three cases above run with
//! `honor_game_colours = true` (the shipped default), and each has a
//! theme-only (`false`) companion pinning where the two modes diverge.

use std::path::PathBuf;

use app::engine::{Engine, ScreenModel, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way the app does. `colours` is `honor_game_colours`,
/// whose config default is `true`.
fn boot_v6(file: &str, colours: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, colours, false, None, false, dims, picts.std_window(), Some((2, 9)), None)
        .expect("v6 story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn state_for(mode: app::config::V6RenderMode) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    st.config.v6_render = mode;
    st
}

/// Render `model` into an 80×30 pane and count cell backgrounds:
/// `(pure-white cells, non-white coloured "art" cells)`. In the halfblock cell
/// path the scaled picture arrives as `Rgb` cell backgrounds, so a healthy v6
/// frame has many art cells; a flooded one is uniformly white.
///
/// The raster composite is resized+encoded on a background worker (SQ-0469) and
/// only appears once `poll_v6_job` installs it, so — like the real app, which
/// polls every frame — this renders until the encode lands before counting.
/// (The hybrid ring's band images are drawn synchronously; the settle only
/// matters for the whole-canvas raster path, which hybrid also falls through to
/// when there is no story window.)
fn bg_census(model: &ScreenModel, mode: app::config::V6RenderMode) -> (usize, usize) {
    let (white, art, _) = cell_census(model, mode);
    (white, art)
}

/// As [`bg_census`], plus the count of cells the render left at `Color::Reset` —
/// the HOST THEME's backdrop, i.e. every cell no game pixel or page reached.
/// (SQ-0532 wave-5, defect 1: with a game-set page that count must be zero.)
fn cell_census(model: &ScreenModel, mode: app::config::V6RenderMode) -> (usize, usize, usize) {
    cell_census_honor(model, mode, true)
}

/// As [`cell_census`] but with the APP-side `honor_game_colours` set explicitly
/// — independent of what the engine recorded (the live `/set-game-colours`
/// toggle changes only the app config, never the model).
fn cell_census_honor(model: &ScreenModel, mode: app::config::V6RenderMode, honor: bool) -> (usize, usize, usize) {
    let mut state = state_for(mode);
    state.config.honor_game_colours = honor;
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    for _ in 0..200 {
        let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
        if !state.graphics_render.borrow().v6_encode_in_flight() {
            break; // this path draws no whole-canvas raster — nothing to wait for
        }
        if state.graphics_render.borrow_mut().poll_v6_job() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    let (mut white, mut art, mut backdrop) = (0, 0, 0);
    for y in 0..area.height {
        for x in 0..area.width {
            match buf.cell((x, y)).unwrap().bg {
                Color::Rgb(255, 255, 255) => white += 1,
                Color::Reset => backdrop += 1,
                _ => art += 1,
            }
        }
    }
    (white, art, backdrop)
}

/// The story window's explicit `(fg, bg)` pair, as the render resolves it.
/// `None` per channel when the game set no colour there.
fn story_pair(model: &ScreenModel) -> (Option<image::Rgba<u8>>, Option<image::Rgba<u8>>) {
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let colors = app::colors::ColorScheme::terminal_default();
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    (
        app::render::v6_layout::story_fg_rgba(layout.story, &colors),
        app::render::v6_layout::story_bg_rgba(layout.story, &colors),
    )
}

/// The story window's native pixel rect `(x, y, w, h)` in the 640×400 canvas.
fn story_rect(model: &ScreenModel) -> (u32, u32, u32, u32) {
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let s = layout.story.expect("a story window");
    (s.x_px as u32, s.y_px as u32, s.w_px as u32, s.h_px as u32)
}

/// The v6 RASTER composite in the game's native pixel space, plus its story
/// scroll/pager metrics. `build_v6_raster_canvas` IS the render's own canvas
/// step (`render_story_pane` calls exactly this and hands the result to the
/// resize+encode worker), so asserting on these pixels pins the shipped
/// composite rather than a re-implementation of it. The pane's own scaling is
/// deliberately not involved: the game's ink, page, glyphs and drop-caps all
/// live here, at native resolution.
fn raster_canvas(
    model: &ScreenModel,
    state: &app::state::AppState,
) -> (image::RgbaImage, Option<app::render::screen::RasterMetrics>) {
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    app::render::screen::build_v6_raster_canvas(&layout, native, state)
}

/// Regression 2 (hybrid). **Mode: `honor_game_colours = true`** — the shipped
/// default, and the only mode in which this ever broke. Zork Zero's frame art
/// must reach the pane. Broken build: 2400 white / 0 art, a solid white pane.
/// Fixed: 0 white / 950 art.
#[test]
fn zork0_hybrid_keeps_its_art_when_game_colours_are_honoured() {
    let Some(mut s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    for _ in 0..3 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit("ne"),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "Zork0 faulted: {:?}", r.fault);
    }
    let (white, art) = bg_census(&s.screen(), app::config::V6RenderMode::Hybrid);
    eprintln!("Zork0 hybrid census: white={white} art={art} (of 2400)");
    assert!(art > 0, "Zork0's frame art vanished from the hybrid pane (white={white}, art={art})");
    assert!(white < 2000, "the hybrid pane is flooded with opaque white (white={white} of 2400)");
}

/// Regression 1 (raster). **Mode: `honor_game_colours = true`.** The whole PANE
/// must not be repainted in the current window's background. Broken build: 2400
/// white cells (every cell of the pane). Fixed: ~1269 — the raster composite's
/// own story page, which legitimately IS the game's white (SQ-0510,
/// `story_bg_rgba`: a game-set story-window colour wins), surrounded by real
/// frame art. The threshold separates "the game's page inside its frame" from
/// "the pane has been flooded".
#[test]
fn zork0_raster_page_is_not_flooded_white_when_game_colours_are_honoured() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let (white, art) = bg_census(&s.screen(), app::config::V6RenderMode::Raster);
    eprintln!("Zork0 raster census: white={white} art={art} (of 2400)");
    assert!(white < 2000, "the raster page is flooded with opaque white (white={white} of 2400)");
    assert!(art > 0, "Zork0's frame art vanished from the raster pane (white={white}, art={art})");
}

/// Paired companion to the two above. **Mode: `honor_game_colours = false`** —
/// the theme-only path, where the game's `set_colour` is declined outright.
/// Both modes must reach the pane with real art and no flood; the DIVERGENCE
/// that must persist is only in the model: with colours honoured the story
/// window carries the game's explicit pair (§8.3), with them declined it stays
/// `Default`. Neither may ever become the pane page.
#[test]
fn zork0_renders_its_frame_in_both_colour_modes() {
    for colours in [true, false] {
        let Some(s) = boot_v6("zork0-r393-s890714.z6", colours) else { return };
        let model = s.screen();
        assert_eq!(
            (model.bg, model.fg),
            (0, 0),
            "colours={colours}: a v6 model never publishes a pane page — the host theme owns it"
        );
        let w0 = &s.machine.screen.v6.as_ref().expect("v6 screen").windows[0];
        if colours {
            assert_ne!(w0.bg, zvm::screen::ZColour::Default, "the game's window-0 background is recorded (§8.3)");
        } else {
            assert_eq!(w0.bg, zvm::screen::ZColour::Default, "colours declined: the game never set one");
        }
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            let (white, art) = bg_census(&model, mode);
            eprintln!("Zork0 {mode:?} colours={colours}: white={white} art={art} (of 2400)");
            assert!(art > 0, "colours={colours} {mode:?}: the frame art vanished (white={white}, art={art})");
            assert!(white < 2000, "colours={colours} {mode:?}: the pane is flooded white ({white} of 2400)");
        }
    }
}

/// The precise mechanism behind both: `erase_window` must not turn a window's
/// whole CHARACTER GRID into opaque explicitly-coloured cells. ZMSD §8.8.5.3.1
/// does say -1 erases the screen to window 0's background — but that is a paint
/// onto the screen/canvas, which the engine already reports separately as a
/// canvas-clear event. Stamping it into the persistent text grid as well makes
/// the grid an opaque layer that the compositor draws OVER every picture, for
/// the rest of the session.
///
/// Zork Zero's window 7 is the full-screen decorative window: after boot its
/// 25×80 grid holds no text at all, yet every one of its 2000 blank cells used
/// to carry an explicit background.
///
/// **Modes: both.** The grid must be transparent either way; the divergence is
/// the window-level pair, which is where §8.8.5.3's erase colour actually lives
/// (together with the number-0 canvas-clear event on the picture queue).
#[test]
fn erase_window_must_not_make_a_blank_text_grid_opaque() {
    for colours in [true, false] {
        let Some(s) = boot_v6("zork0-r393-s890714.z6", colours) else { return };
        let v6 = s.machine.screen.v6.as_ref().expect("v6 screen");
        let w7 = &v6.windows[7];
        assert_eq!((w7.x_size, w7.y_size), (640, 400), "window 7 is Zork0's full-screen decorative window");
        assert!(w7.texts.is_empty(), "it holds no painted text — it is pure backdrop");
        let explicit = w7
            .grid
            .cells
            .iter()
            .filter(|c| c.bg != zvm::screen::ZColour::Default)
            .count();
        eprintln!(
            "Zork0 window 7 (colours={colours}): {explicit} of {} blank cells carry an explicit bg",
            w7.grid.cells.len()
        );
        assert_eq!(
            explicit, 0,
            "colours={colours}: a blank erased cell must stay transparent to the compositor, or its \
             window covers the art beneath it ({explicit} of {} cells are opaque)",
            w7.grid.cells.len()
        );
        // The divergence: the erase colour still reaches the WINDOW when the
        // game's colours are honoured — it just never reaches the grid cells.
        if colours {
            assert_eq!(w7.bg, zvm::screen::ZColour::Standard(9), "the game's window-7 background is kept (§8.3)");
        } else {
            assert_eq!(w7.bg, zvm::screen::ZColour::Default, "colours declined: no window background is set");
        }
    }
}

/// Regression 3. Shogun's title splash is a full-screen picture drawn before the
/// game splits any window. It must keep reaching the hybrid pane.
///
/// **Modes: both** — this one is purely the window-0 boot-geometry change and
/// reproduced with `honor_game_colours` either way, so it is driven in both and
/// the two must agree (Shogun sets no colour at all on this screen).
///
/// The wave-1 §8.8.3.3 fix itself is NOT reverted: window 0 really does report
/// the whole 640×400 screen to `get_wind_prop` (asserted below). It simply must
/// not be RENDERED as the story page while it is an empty placeholder.
#[test]
fn shogun_splash_still_shows_its_art_in_hybrid() {
    for colours in [true, false] {
        let Some(s) = boot_v6("shogun-r322-s890706.z6", colours) else { return };

        // The spec win survives: the model still holds a whole-screen window 0.
        let w0 = &s.machine.screen.v6.as_ref().expect("v6 screen").windows[0];
        assert_eq!(
            (w0.x_size, w0.y_size),
            (640, 400),
            "§8.8.3.3: window 0 still occupies the whole screen in the model"
        );

        // Structural mechanism: an empty whole-screen window 0 must not become
        // the story page, or hybrid carves a pane-sized transcript viewport over
        // the splash and the chrome ring collapses to zero thickness.
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        if let Some(story) = layout.story {
            assert!(
                !(story.w_px >= native.0 && story.h_px >= native.1),
                "colours={colours}: before the game splits anything, window 0 covers the entire \
                 {native:?} screen ({}x{}), so hybrid renders a story page over the splash instead \
                 of showing the art",
                story.w_px,
                story.h_px
            );
        }

        let (white, art) = bg_census(&model, app::config::V6RenderMode::Hybrid);
        eprintln!("Shogun splash hybrid census (colours={colours}): white={white} art={art} (of 2400)");
        assert!(
            art > 200,
            "colours={colours}: Shogun's splash art vanished from the hybrid pane (white={white}, art={art})"
        );
    }
}

/// The other half of fix 3: once the game GIVES window 0 something, it becomes
/// the story window again — the skip is a placeholder rule, not a removal.
/// **Modes: both**, since it is geometry/content driven, not colour driven.
#[test]
fn shogun_gets_its_story_window_back_once_the_game_uses_it() {
    for colours in [true, false] {
        let Some(mut s) = boot_v6("shogun-r322-s890706.z6", colours) else { return };
        for _ in 0..6 {
            let r = match s.pending_input() {
                InputKind::Line => s.submit("look"),
                InputKind::Char => s.submit_char(13),
                InputKind::Event => s.submit(""),
            };
            assert!(r.fault.is_none(), "Shogun faulted: {:?}", r.fault);
        }
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("colours={colours}: in play, window 0 IS the story window");
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        assert!(
            story.w_px < native.0 || story.h_px < native.1,
            "colours={colours}: and it is the game's own box, not the whole screen"
        );
    }
}

// ── SQ-0532 wave-5: the three defects the wave-4 build still showed on a TTY ──
//
// All three are Zork Zero with `honor_game_colours = true` (the shipped default,
// and the mode the user runs): its boot `set_colour(fg=2 black, bg=9 white)` on
// window 0 gives it an explicit §8.3 pair, which the render must honour as a
// PAIR and as the whole screen's page — the DOS original is a white page with
// the frame art printed on it, not white art floating on a dark host theme.

/// Wave-5 defect 1 (hybrid). **Mode: `honor_game_colours = true`.** The game's
/// page covers the ENTIRE pane — letterbox margins, the chrome band surrounds,
/// everything behind the frame art — not just the story-window region. Broken
/// build: the letterbox/backdrop cells stayed `Color::Reset` (the host theme),
/// so Zork Zero's white frame floated on a dark surround.
#[test]
fn zork0_hybrid_page_covers_the_whole_pane() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let model = s.screen();
    // Premise: the game really did set a page on its story window.
    let (fg, bg) = story_pair(&model);
    assert_eq!(bg, Some(image::Rgba([255, 255, 255, 255])), "Zork0's window 0 is explicit white");
    assert_eq!(fg, Some(image::Rgba([0, 0, 0, 255])), "…on explicit black ink");
    let (white, art, backdrop) = cell_census(&model, app::config::V6RenderMode::Hybrid);
    eprintln!("Zork0 hybrid page census: white={white} art={art} backdrop={backdrop} (of 2400)");
    assert_eq!(
        backdrop, 0,
        "every pane cell must carry the game's page or its art — {backdrop} cells were left on the \
         host theme backdrop, which is the white-frame-on-dark-surround defect"
    );
    assert!(art > 0, "the frame art still reaches the pane (white={white}, art={art})");
}

/// Paired companion. **Mode: `honor_game_colours = false`** — the game's
/// `set_colour` is declined outright, so the story window has NO explicit page
/// and the host theme owns the pane exactly as before. This is the divergence
/// the whole-pane fill must be strictly gated on.
#[test]
fn zork0_hybrid_keeps_the_theme_backdrop_when_colours_are_declined() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", false) else { return };
    let model = s.screen();
    assert_eq!(story_pair(&model), (None, None), "colours declined: the window carries no pair");
    let (_, _, backdrop) = cell_census(&model, app::config::V6RenderMode::Hybrid);
    eprintln!("Zork0 hybrid backdrop with colours declined: {backdrop} (of 2400)");
    assert!(
        backdrop > 0,
        "with no game page the host theme must still own the pane surround; flooding it anyway \
         would repaint every v6 game's letterbox"
    );
}

/// The LIVE toggle (`/set-game-colours off`) flips only the APP config — the
/// engine model still carries the white pair Zork Zero recorded at boot. The
/// render must gate the page/ink fills on the live config, not on what the
/// model recorded, or the toggle recolours the text but leaves the white
/// backdrop standing (SQ-0532 TTY-pass finding, 2026-07-28). Boots with the
/// engine HONORING (pair present), then renders with the app config declined.
#[test]
fn live_toggle_off_repaints_backdrop_and_raster_page_without_reboot() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let model = s.screen();
    let (_, bg) = story_pair(&model);
    assert!(bg.is_some(), "engine honored at boot: the story window carries its white page");

    // Hybrid: with the app config declined the whole-pane page fill must not
    // run — the theme backdrop owns the letterbox again.
    let (_, _, backdrop) = cell_census_honor(&model, app::config::V6RenderMode::Hybrid, false);
    assert!(
        backdrop > 0,
        "app-side honor off must skip the page flood even though the model still carries white"
    );

    // Raster: the composite's story page must fall back to the theme default,
    // not the model's white.
    let mut st = state_for(app::config::V6RenderMode::Raster);
    st.config.honor_game_colours = false;
    let (canvas, _) = raster_canvas(&model, &st);
    let (sx, sy, sw, sh) = story_rect(&model);
    let p = canvas.get_pixel(sx + sw / 2, sy + sh / 2);
    assert_ne!(
        (p[0], p[1], p[2]),
        (255, 255, 255),
        "app-side honor off must not paint the raster story page white"
    );
}

/// The other half of the gate: a game that sets NO story-window colour keeps
/// today's look even with `honor_game_colours = true`. Arthur draws a header art
/// panel and side borders and never calls `set_colour`, so its pane surround must
/// stay the theme backdrop — the fill is keyed on the window's explicit bg, not
/// on the flag.
#[test]
fn a_game_that_sets_no_page_keeps_the_theme_backdrop() {
    let Some(s) = boot_v6("arthur-r74-s890714.z6", true) else { return };
    let model = s.screen();
    let (_, bg) = story_pair(&model);
    assert_eq!(bg, None, "Arthur sets no story-window background");
    let (_, _, backdrop) = cell_census(&model, app::config::V6RenderMode::Hybrid);
    eprintln!("Arthur hybrid backdrop (colours honoured): {backdrop} (of 2400)");
    assert!(backdrop > 0, "Arthur's pane surround must stay the host theme backdrop");
}

/// Wave-5 defect 2 (raster). **Mode: `honor_game_colours = true`.** The story
/// prose must be rasterized in the GAME's ink, not the host's resolved default.
/// Broken build: `draw_story_text` was handed `default_fg` (the theme/OSC ink,
/// (220,220,220) here) while the page took the game's white — light grey on
/// white, i.e. invisible text. Fixed: the window's explicit fg wins over
/// `default_fg` exactly as its bg already wins over `default_bg`.
///
/// Asserted on the composite the render itself builds, so this pins the shipped
/// pixels rather than a re-implementation.
#[test]
fn zork0_raster_inks_its_prose_in_the_games_own_colour() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let model = s.screen();
    let (sx, sy, sw, sh) = story_rect(&model);
    let mut state = state_for(app::config::V6RenderMode::Raster);
    state.push_transcript("The hall is filled to capacity.");
    // Premise: the host's own default ink is the light grey that used to be used
    // for the prose — the colour that must NOT appear on the game's white page.
    assert_eq!(
        app::render::screen::v6_host_pair(&state).0,
        image::Rgba([220, 220, 220, 255]),
        "the host default ink under this theme is light grey"
    );
    let (img, _) = raster_canvas(&model, &state);

    // Inside the story box the only things drawn are the page, the glyphs and any
    // inline float — the frame art is inset out by `story_clear_native`.
    let mut page = 0usize;
    let mut ink = 0usize;
    let mut default_ink = 0usize;
    for y in sy..(sy + sh).min(img.height()) {
        for x in sx..(sx + sw).min(img.width()) {
            match img.get_pixel(x, y).0 {
                [255, 255, 255, 255] => page += 1,
                [0, 0, 0, 255] => ink += 1,
                [220, 220, 220, 255] => default_ink += 1,
                _ => {}
            }
        }
    }
    eprintln!("Zork0 raster story box: page={page} ink={ink} host-default-ink={default_ink}");
    assert!(page > 1000, "the story page is the game's white ({page} px)");
    assert!(ink > 100, "the prose is inked in the game's black ({ink} px)");
    assert_eq!(
        default_ink, 0,
        "no glyph may still be drawn in the host's default ink — that is the white-on-white defect \
         ({default_ink} px of (220,220,220) inside the story box)"
    );
}

/// Paired companion. **Mode: `honor_game_colours = false`.** With the game's
/// colours declined the window carries no pair at all, so BOTH channels fall back
/// to the host's resolved default pair — the ink is the host's (220,220,220) and
/// the page its black. Ink and page still differ; they simply come from the host.
#[test]
fn zork0_raster_falls_back_to_the_host_pair_when_colours_are_declined() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", false) else { return };
    let model = s.screen();
    let (sx, sy, sw, sh) = story_rect(&model);
    assert_eq!(story_pair(&model), (None, None), "colours declined: no game pair to honour");
    let mut state = state_for(app::config::V6RenderMode::Raster);
    state.push_transcript("The hall is filled to capacity.");
    let (img, _) = raster_canvas(&model, &state);
    let mut default_ink = 0usize;
    for y in sy..(sy + sh).min(img.height()) {
        for x in sx..(sx + sw).min(img.width()) {
            if img.get_pixel(x, y).0 == [220, 220, 220, 255] {
                default_ink += 1;
            }
        }
    }
    eprintln!("Zork0 raster host-default ink with colours declined: {default_ink} px");
    assert!(
        default_ink > 100,
        "with no game ink the host default must still draw the prose ({default_ink} px)"
    );
}

/// Wave-5 defect 3 (raster). Zork Zero's illuminated opening drop-cap must be on
/// screen when the game starts.
///
/// Root cause it guards: the opening banner wraps to more rows than Zork Zero's
/// 20-row window 0 holds, so the view pinned to the NEWEST rows and the drop-cap
/// — anchored to the very first transcript line — was cropped away before the
/// player ever saw it. The `[more]` pager (SQ-0404) is exactly the machinery for
/// "one batch of output overflowed the story box"; `startup` now arms it for the
/// opening banner as `finish_command_turn` already does for a turn, so the first
/// frame parks the view on the first screenful. This drives that same decision —
/// `activation_target(0, total, viewport)` — and then asserts the drop-cap's own
/// pixels are really in the composite.
#[test]
fn zork0_raster_shows_its_opening_drop_cap() {
    use app::engine::Engine;
    use app::state::apply_transcript_elems;

    let Some(mut s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let mut state = state_for(app::config::V6RenderMode::Raster);
    // The REAL elems pipeline: the banner's window-0 pictures arrive as ordered
    // `TranscriptElem::Image`s and land in `transcript_images`.
    let elems = Engine::take_transcript_elems(&mut s);
    apply_transcript_elems(&mut state, &elems);
    let drop_cap = state.transcript_images[0]
        .clone()
        .expect("Zork0's opening line IS the illuminated drop-cap");
    assert_eq!(drop_cap.align, app::inline_image::ImageAlign::MarginLeft, "it floats at the left margin");

    let model = s.screen();
    let (sx, sy, ..) = story_rect(&model);

    // Frame 1: measure. The banner overflowed its story box, so the pager engages
    // — this is exactly the decision `startup`'s arm hands the first frame.
    let (before, m) = raster_canvas(&model, &state);
    let m = m.expect("Zork0 has a story window");
    eprintln!("Zork0 banner: total_rows={} viewport_rows={}", m.total_rows, m.viewport_rows);
    assert!(
        m.total_rows > m.viewport_rows,
        "premise: the opening banner ({} rows) really does overflow the {}-row story box",
        m.total_rows,
        m.viewport_rows
    );
    // Broken build: pinned to the newest rows, the drop-cap is cropped away.
    let unparked = drop_cap_pixels(&before, &drop_cap, sx, sy);
    // `prompt_rows = 0`: the raster path stamps its `[more]` over the tail of the
    // last prose row instead of reserving one, so its viewport doesn't shrink.
    let target = app::pager::activation_target(0, m.total_rows, m.viewport_rows, 0).expect(
        "Zork0's opening banner overflows its 20-row story box — the pager must engage, or the \
         drop-cap on the first row is scrolled away unread",
    );
    state.transcript_scroll = target;
    state.pager.active = true;

    // Frame 2: the parked view must show the drop-cap, whole.
    let (img, _) = raster_canvas(&model, &state);
    let (matched, opaque) = drop_cap_pixels(&img, &drop_cap, sx, sy);
    let (dw, dh) = drop_cap.pixels.dimensions();
    eprintln!("Zork0 drop-cap pixels in the composite: parked {matched}/{opaque}, unparked {}/{}", unparked.0, unparked.1);
    assert!(opaque > 1000, "the drop-cap art has substance ({opaque} opaque px of {dw}x{dh})");
    assert!(
        matched * 10 >= opaque * 9,
        "the drop-cap must be blitted into the story box at the parked scroll — only {matched} of \
         {opaque} of its pixels reached the composite"
    );
    // And the defect it replaces really was a defect: pinned to the newest rows,
    // almost none of it survived (the float is cropped from its own top).
    assert!(
        unparked.0 * 2 < opaque,
        "premise: unparked, the drop-cap is cropped away ({} of {opaque} px survived) — if it were \
         already whole this test would prove nothing",
        unparked.0
    );
}

/// `(matching, opaque)` pixel counts for `img`'s copy of `drop_cap` blitted with
/// its top-left at native `(ox, oy)` — how much of the picture actually reached
/// the composite. Transparent source pixels are not counted (nothing to blit).
fn drop_cap_pixels(
    img: &image::RgbaImage,
    drop_cap: &app::inline_image::InlineImage,
    ox: u32,
    oy: u32,
) -> (usize, usize) {
    let (dw, dh) = drop_cap.pixels.dimensions();
    let (mut opaque, mut matched) = (0usize, 0usize);
    for y in 0..dh {
        for x in 0..dw {
            let src = drop_cap.pixels.get_pixel(x, y);
            if src[3] < 128 {
                continue;
            }
            opaque += 1;
            let (cx, cy) = (ox + x, oy + y);
            if cx < img.width() && cy < img.height() && img.get_pixel(cx, cy).0[..3] == src.0[..3] {
                matched += 1;
            }
        }
    }
    (matched, opaque)
}

// ── SQ-0532 wave-6: the TYPED input takes the game's pair too ─────────────────
//
// The live input line resolves its colour through `game_input_style`, which read
// `ScreenModel.bg/fg` — the PANE PAGE. Wave 4 correctly stopped a v6 model from
// publishing one (a v6 story has none; every window carries its own §8.3 pair),
// so the typed text fell back to the theme's grey `input_text` while the prose
// beside it — coloured per-run from the same §8.3 mirror — stayed black. On Zork
// Zero's white page that is grey-on-white: the user's TTY report. The committed
// echo was never affected (it inherits the prompt line's runs, SQ-0269), which is
// exactly the divergence pinned below.

/// The typed text's drawn cells: find `needle` in the rendered pane and return
/// the cells of its first char and of the one just past it (the caret, when
/// `needle` is the whole live input). Panics if the text never reached the pane.
fn drawn_cells(buf: &Buffer, area: Rect, needle: &str) -> (ratatui::buffer::Cell, ratatui::buffer::Cell) {
    for y in 0..area.height {
        let row: String = (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
        if let Some(byte_at) = row.find(needle) {
            let x = row[..byte_at].chars().count() as u16;
            let after = x + needle.chars().count() as u16;
            return (
                buf.cell((x, y)).unwrap().clone(),
                buf.cell((after.min(area.width - 1), y)).unwrap().clone(),
            );
        }
    }
    panic!("{needle:?} was never drawn into the pane");
}

/// Render `model` in hybrid with `typed` sitting at the live prompt, mirroring the
/// game's own turn output (with its §8.3 runs) into the transcript first, exactly
/// as `turn.rs` does. Returns the pane so the caller can find its cells.
fn hybrid_pane_with_input(
    model: &ScreenModel,
    colours: bool,
    turn: &app::session::TurnResult,
    echo: Option<&str>,
    typed: &str,
    area: Rect,
) -> Buffer {
    let mut state = state_for(app::config::V6RenderMode::Hybrid);
    state.config.honor_game_colours = colours;
    state.push_transcript_runs(&turn.transcript, app::state::TranscriptKind::Story, &turn.transcript_runs);
    if state.transcript.is_empty() {
        state.push_transcript(">"); // a turn that printed nothing still has a prompt to type at
    }
    if let Some(cmd) = echo {
        // Inline mode: the game's own `>` is the last transcript line, so the
        // committed command is appended to it (turn.rs's echo path).
        state.append_to_last_transcript_line(cmd);
    }
    state.input.set(typed, true);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    buf
}

/// Play a v6 story to its first ordinary line prompt and return the session plus
/// that turn's output (whose style runs carry the game's §8.3 pair).
fn v6_at_prompt(file: &str, colours: bool) -> Option<(GameSession, app::session::TurnResult)> {
    let mut s = boot_v6(file, colours)?;
    let mut last = None;
    for _ in 0..6 {
        let r = match s.pending_input() {
            InputKind::Line => {
                last = Some(s.submit("look"));
                break;
            }
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "{file} faulted: {:?}", r.fault);
        last = Some(r);
    }
    let turn = last.expect("the story produced a turn");
    Some((s, turn))
}

/// The theme's own input-text style — what the typed line must fall back to
/// whenever there is no game pair to honour.
fn theme_input_style() -> ratatui::style::Style {
    let c = app::colors::ColorScheme::terminal_default();
    c.theme.get("transcript").style.patch(c.theme.get("input_text").style)
}

/// **Mode: `honor_game_colours = true`.** The live, being-typed input renders in
/// Zork Zero's own pair — black ink on its white page — like the prose beside it.
/// Broken build: the theme's grey `input_text` on the game's white page.
#[test]
fn zork0_hybrid_types_the_live_input_in_the_games_own_colour() {
    let Some((s, turn)) = v6_at_prompt("zork0-r393-s890714.z6", true) else { return };
    let model = s.screen();
    assert_eq!(story_pair(&model).1, Some(image::Rgba([255, 255, 255, 255])), "premise: Zork0's page is white");
    let palette = app::colors::ColorScheme::terminal_default().palette;
    let (black, white) = (palette[0], palette[7]); // Z Standard(2) / Standard(9)
    let area = Rect::new(0, 0, 80, 30);
    let buf = hybrid_pane_with_input(&model, true, &turn, None, "xyzzyq", area);
    let (text, caret) = drawn_cells(&buf, area, "xyzzyq");
    eprintln!("Zork0 hybrid live input: fg={:?} bg={:?}; caret fg={:?} bg={:?} rev={}",
        text.fg, text.bg, caret.fg, caret.bg, caret.modifier.contains(ratatui::style::Modifier::REVERSED));
    assert_eq!(text.fg, black, "the typed text must be inked in the game's black, not the theme's grey");
    assert_eq!(text.bg, white, "…on the game's white page");
    assert_ne!(Some(text.fg), theme_input_style().fg, "premise: the theme's input ink is NOT the game's");
    // The caret is reverse-video of the resolved input style, so it stays a
    // visible block on the game's page (a bare theme cursor would not be).
    assert!(caret.modifier.contains(ratatui::style::Modifier::REVERSED), "the caret must reverse the game pair");
    assert_eq!((caret.fg, caret.bg), (black, white), "…reversing the GAME's pair, not the theme's");
}

/// Paired companion. **Mode: `honor_game_colours = false`.** With the game's
/// `set_colour` declined there is no pair to honour, so the typed line keeps the
/// theme's own input style exactly as before.
#[test]
fn zork0_hybrid_types_the_live_input_in_the_theme_when_colours_are_declined() {
    let Some((s, turn)) = v6_at_prompt("zork0-r393-s890714.z6", false) else { return };
    let model = s.screen();
    assert_eq!(story_pair(&model), (None, None), "colours declined: no game pair exists");
    let area = Rect::new(0, 0, 80, 30);
    let buf = hybrid_pane_with_input(&model, false, &turn, None, "xyzzyq", area);
    let (text, _) = drawn_cells(&buf, area, "xyzzyq");
    eprintln!("Zork0 hybrid live input, colours declined: fg={:?} bg={:?}", text.fg, text.bg);
    let theme = theme_input_style();
    assert_eq!(Some(text.fg), theme.fg, "the theme's input ink owns the typed line");
    assert_ne!(text.bg, app::colors::ColorScheme::terminal_default().palette[7], "no game page is painted under it");
}

/// The other half of the gate: a v6 game that sets NO story-window colour keeps
/// the theme's input style even with colours honoured — Arthur never calls
/// `set_colour`, so its typed line must look exactly as it does today.
#[test]
fn a_game_that_sets_no_page_types_the_live_input_in_the_theme() {
    let Some((s, turn)) = v6_at_prompt("arthur-r74-s890714.z6", true) else { return };
    let model = s.screen();
    assert_eq!(story_pair(&model), (None, None), "Arthur sets no story-window pair");
    let area = Rect::new(0, 0, 80, 30);
    let buf = hybrid_pane_with_input(&model, true, &turn, None, "xyzzyq", area);
    let (text, _) = drawn_cells(&buf, area, "xyzzyq");
    eprintln!("Arthur hybrid live input (colours honoured): fg={:?} bg={:?}", text.fg, text.bg);
    assert_eq!(Some(text.fg), theme_input_style().fg, "with no game pair the theme still owns the typed line");
}

/// The committed echo (`>look` in scrollback) already resolved the game's pair —
/// it inherits the prompt line's trailing run (SQ-0269) — and must keep doing so,
/// in the SAME pair the live line now uses. Pinning both together is what stops
/// the two halves of one prompt from diverging again.
#[test]
fn zork0_hybrid_echoes_the_committed_command_in_the_live_inputs_pair() {
    let Some((s, turn)) = v6_at_prompt("zork0-r393-s890714.z6", true) else { return };
    let model = s.screen();
    let area = Rect::new(0, 0, 80, 30);
    let palette = app::colors::ColorScheme::terminal_default().palette;
    let (black, white) = (palette[0], palette[7]);
    // The committed echo of "xyzzyq" in scrollback, with a fresh "plugh" being typed.
    let buf = hybrid_pane_with_input(&model, true, &turn, Some("xyzzyq"), "plugh", area);
    let (echo, _) = drawn_cells(&buf, area, "xyzzyq");
    let (live, _) = drawn_cells(&buf, area, "plugh");
    eprintln!("Zork0 hybrid echo: fg={:?} bg={:?}; live: fg={:?} bg={:?}", echo.fg, echo.bg, live.fg, live.bg);
    assert_eq!((echo.fg, echo.bg), (black, white), "the committed echo keeps the game's pair");
    assert_eq!((live.fg, live.bg), (echo.fg, echo.bg), "the live line and its committed echo are one prompt");
}
