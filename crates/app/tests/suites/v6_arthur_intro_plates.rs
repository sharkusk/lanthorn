//! SQ-0695 — Arthur's intro illustrations must rasterize at the position the
//! game declared, and composite in place.
//!
//! Arthur opens with three illustrated screens. For each one it erases every
//! window, asks window 0 for its OWN size (`get_wind_prop` props 2 and 3 → 400
//! and 640), centres the plate itself — x = (640 − 584)/2 + 1 = **29**,
//! y = (400 − 392)/2 + 1 = **5** — and draws it there. The Merlin screen redraws
//! the same graveyard plate at that same origin and composites picture 3 (480×300
//! at (81,51)) *inside* it, so Merlin appears on the graveyard within one frame.
//!
//! An illustrated screen carries NO prose (SQ-0707). The measured op sequence is
//! `@erase_window(-1)` → `@draw_picture` → `@set_cursor(row=-1)` (ZMSD §8.8.3.4:
//! hide the cursor) → a timed `@read_char`; the whole graveyard→Merlin turn is 31
//! instructions and prints not one character. The narration is a SEPARATE screen
//! that the game erases before printing and erases again before the next plate.
//! So a plate frame's prose is empty, and rasterizing the app's scrollback onto
//! the plate paints the PREVIOUS screen's text across the artwork.
//!
//! What went wrong: `GameSession::apply_picture_event` treated EVERY window-0
//! draw as inline story content — Zork Zero's drop-caps and room icons, which
//! anchor to the transcript at their output-char position and scroll with the
//! text. Arthur's absolutely-placed backdrops were pushed into `story_pics` as
//! transcript floats, so `pictures_canvas` stayed EMPTY for the whole intro, the
//! model published no `Graphics` leaf, and the art never rasterized at all. The
//! two plates would also have stacked as separate full-width bands rather than
//! compositing.
//!
//! The discriminator is the one the game itself supplies, and it is measured, not
//! guessed: a picture drawn on window 0's CURRENT TEXT LINE flows with the text;
//! one placed anywhere else is absolutely placed. Across the corpus —
//!
//! | game      | draw                        | cursor y | verdict  |
//! |-----------|-----------------------------|----------|----------|
//! | Zork Zero | pic 2 at y=17, 216 at y=193 | 17, 193  | inline   |
//! | Shogun    | pic 7, raw y=0 → y=1        | 1        | inline   |
//! | Arthur    | pic 2 at y=5, pic 3 at y=51 | 1        | absolute |
//!
//! — only the VERTICAL axis counts, because an inline float is anchored to the
//! text's own line; its x is a margin choice (Shogun's ship sits at x=229 to
//! reserve the right column). `PictureEvent::at_cursor` carries that from the
//! engine, which is the only place it survives: the engine resolves a `0` operand
//! to the cursor before publishing, so by the time the host sees `y` the two are
//! indistinguishable.
//!
//! The render half was broken independently: `classify_windows` has always set a
//! `WinNode::Graphics` whose `win == 0` aside as `story_gfx` — story content, not
//! chrome — but **nothing ever drew it**. It was classified and dropped. So even
//! a correct canvas would have rendered nothing.
//!
//! Both v6 pixel modes are pinned. HYBRID is the default and what the player
//! sees: an intro screen has no chrome ring at all, so it must take the
//! `picture_takeover` fall-through to the raster composite (SQ-0570) — which
//! ships the plate as one image — rather than opening a pane-wide transcript
//! viewport over art it never uploads. RASTER ships the same composite directly.
//! Both `honor_game_colours` modes are pinned too (`true` is the shipped
//! default): the declared placement is the game's geometry, not its palette, so
//! it must be identical under both.
//!
//! The story asset is gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Arthur's own centring of the 584×392 plate in its 640×400 window 0, as ZMSD
/// §8.8.1 1-based window coords: `@draw_picture(number=2, window=0, y=5, x=29)`.
/// The canvas is 0-based, so the plate occupies x 28..612, y 4..396.
const PLATE: (u32, u32, u32, u32) = (28, 4, 584, 392);
/// Picture 3 (Merlin, 480×300) at `@draw_picture(number=3, window=0, y=51, x=81)`
/// → 0-based x 80..560, y 50..350. Wholly inside [`PLATE`].
const MERLIN: (u32, u32, u32, u32) = (80, 50, 480, 300);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Every window-0 canvas Arthur puts up during its intro, in order — one per
/// illustrated screen, captured at the keypress wait where the plate is on
/// screen. Empty vec (via `None`) when the gitignored fixture is absent.
///
/// The intro is a `read_char` sequence: erase every window, draw the plate, hide
/// the cursor, WAIT, show the cursor, erase, narrate. Each pending-input boundary
/// is therefore exactly one illustrated screen.
fn arthur_intro_plates() -> Option<(Vec<image::RgbaImage>, GameSession)> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut plates = Vec::new();
    if let Some(c) = session.pictures_canvas.get(&0) {
        plates.push((*c.img).clone());
    }
    // Tap through the intro; the restore prompt rejects a bare Enter and loops on
    // "Please press Y or N>", so it is answered explicitly. Stop on the MERLIN
    // screen (the third plate) so the returned session is parked on an intro
    // frame with a plate up — the render assertions need one on screen, and one
    // tap further on Arthur splits into its gameplay frame and window 0's canvas
    // is gone.
    for _ in 0..8 {
        if plates.len() >= 3 {
            break;
        }
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
        if let Some(c) = session.pictures_canvas.get(&0) {
            plates.push((*c.img).clone());
        }
    }
    Some((plates, session))
}

/// The bounding box of every pixel any layer painted (`alpha != 0`), as
/// `(x, y, w, h)` — `None` when the image is wholly transparent.
fn opaque_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, px) in img.enumerate_pixels() {
        if px.0[3] != 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0 != u32::MAX).then(|| (x0, y0, x1 - x0, y1 - y0))
}

fn render_state(mode: app::config::V6RenderMode, honor_game_colours: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor_game_colours;
    state
}

// ── The model half: the plate rasterizes, at the origin the game declared ─────

/// Every intro screen puts a window-0 canvas up, and the art lands EXACTLY in the
/// 584×392 box Arthur centred for itself — the margin it deliberately left is
/// still transparent on all four sides. Before the fix `pictures_canvas` was
/// empty for the whole intro and there was no canvas to measure.
#[test]
fn arthur_intro_plates_land_at_the_origin_the_game_centred() {
    let Some((plates, _session)) = arthur_intro_plates() else { return };
    assert!(
        plates.len() >= 3,
        "Arthur's intro puts up a title plate, the graveyard and Merlin — got {} window-0 canvases",
        plates.len()
    );
    for (i, img) in plates.iter().enumerate() {
        assert_eq!(
            (img.width(), img.height()),
            (640, 400),
            "plate {i}'s canvas is window 0's own 640×400 box"
        );
        assert_eq!(
            opaque_bbox(img),
            Some(PLATE),
            "plate {i} must occupy exactly the box Arthur centred (x=29, y=5, 584×392, 1-based) \
             — the placement it computed from `get_wind_prop`, not a transcript float's flow position"
        );
        // The centring margin is the game's own choice and must stay clear.
        for &(x, y) in &[(10u32, 10u32), (620, 10), (10, 390), (620, 390), (320, 1), (320, 398)] {
            assert_eq!(
                img.get_pixel(x, y).0[3],
                0,
                "plate {i}: ({x},{y}) is outside the declared plate and must stay unpainted"
            );
        }
    }
}

/// The Merlin screen redraws the graveyard plate at the same origin and
/// composites picture 3 INSIDE it — one canvas, in place — rather than stacking
/// two full-width bands the way two transcript floats would.
///
/// The graveyard is NOT byte-identical across the two frames: Arthur runs an
/// adaptive palette, so redrawing picture 2 under the palette picture 3
/// installed legitimately recolours it (~17 % of the pixels outside Merlin's
/// rect shift hue). What must hold is geometric — the painted FOOTPRINT never
/// moves or grows, and the change is overwhelmingly concentrated inside picture
/// 3's declared rect, which is where Merlin was painted.
#[test]
fn arthur_composites_merlin_inside_the_graveyard_in_one_frame() {
    let Some((plates, _session)) = arthur_intro_plates() else { return };
    let (mx, my, mw, mh) = MERLIN;
    let (px0, py0, pw, ph) = PLATE;
    // Picture 3's rect is wholly inside picture 2's — that is what "composites on
    // top of the graveyard, in place" means.
    assert!(
        mx >= px0 && my >= py0 && mx + mw <= px0 + pw && my + mh <= py0 + ph,
        "picture 3's rect {MERLIN:?} sits wholly inside the graveyard plate {PLATE:?}"
    );

    let (graveyard, merlin) = (&plates[plates.len() - 2], &plates[plates.len() - 1]);
    // Every intro frame is ONE plate-sized opaque region. Two stacked bands (the
    // pre-fix transcript-float behaviour) could not produce this footprint.
    assert_eq!(opaque_bbox(graveyard), Some(PLATE), "the graveyard frame keeps its declared box");
    assert_eq!(opaque_bbox(merlin), Some(PLATE), "compositing Merlin does not move or grow it");
    assert!(
        (my..my + mh).all(|y| (mx..mx + mw).all(|x| merlin.get_pixel(x, y).0[3] == 255)),
        "picture 3's whole rect is painted inside the graveyard plate"
    );

    // Where the two frames differ: inside Merlin's rect almost everything changed
    // (his art replaced the graveyard there); outside it only the palette drifted.
    let mut inside = (0usize, 0usize);
    let mut outside = (0usize, 0usize);
    for (x, y, px) in graveyard.enumerate_pixels() {
        if px.0[3] == 0 {
            continue;
        }
        let differs = px.0 != merlin.get_pixel(x, y).0;
        let slot = if x >= mx && x < mx + mw && y >= my && y < my + mh { &mut inside } else { &mut outside };
        slot.0 += usize::from(differs);
        slot.1 += 1;
    }
    let pct = |(d, n): (usize, usize)| d * 100 / n.max(1);
    assert!(
        pct(inside) > 75,
        "Merlin's art must replace the graveyard inside picture 3's rect: only {}% of {} pixels changed",
        pct(inside),
        inside.1
    );
    assert!(
        pct(outside) < 40,
        "the graveyard OUTSIDE picture 3's rect must survive the composite (only the adaptive \
         palette may shift it): {}% of {} pixels changed — that reads as a full-frame repaint",
        pct(outside),
        outside.1
    );
    assert!(
        pct(inside) > pct(outside) * 2,
        "the change must be concentrated where picture 3 was declared: inside {}% vs outside {}%",
        pct(inside),
        pct(outside)
    );
}

/// The plate reaches the RENDER MODEL as `story_gfx` — the window-0 graphics leaf
/// `classify_windows` reserves for story content — at window 0's own rect.
#[test]
fn arthur_intro_publishes_the_plate_as_story_graphics() {
    for honor in [true, false] {
        let Some((_plates, session)) = arthur_intro_plates() else { return };
        let _ = honor;
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 publishes a layered composite") };
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        // The capture loop ends on an intro keypress wait, so a plate is up.
        let gfx = layout
            .story_gfx
            .expect("an absolutely-placed window-0 picture publishes a Graphics leaf as story_gfx");
        assert_eq!(
            (gfx.x_px, gfx.y_px, gfx.w_px, gfx.h_px),
            (0, 0, 640, 400),
            "story_gfx carries window 0's own box"
        );
        assert!(layout.story.is_some(), "window 0 is still the story window — the plate is its backdrop");
    }
}

// ── The render half: story_gfx is actually drawn, in both pixel modes ─────────

/// RASTER: the composite carries the plate at its declared origin, with the
/// centring margin resolved to the page (never left transparent for a compositor
/// to colour in, SQ-0510) — and the plate is NOT wiped by the story page fill.
#[test]
fn arthur_raster_composite_carries_the_plate() {
    for honor in [true, false] {
        let Some((_plates, session)) = arthur_intro_plates() else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let state = render_state(app::config::V6RenderMode::Raster, honor);
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let plate = layout.story_gfx.expect("a plate is up").clone();
        let WinNode::Graphics(gw) = &plate.node else { panic!("story_gfx is a Graphics leaf") };

        let (canvas, _metrics) =
            app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        // Every painted pixel of the plate reaches the composite unchanged, at the
        // very coordinates the game named.
        let (px0, py0, pw, ph) = PLATE;
        let mut checked = 0usize;
        for y in (py0..py0 + ph).step_by(17) {
            for x in (px0..px0 + pw).step_by(13) {
                let src = gw.canvas.get_pixel(x, y).0;
                if src[3] == 0 {
                    continue;
                }
                assert_eq!(
                    canvas.get_pixel(x, y).0,
                    src,
                    "honor={honor}: the plate's pixel at ({x},{y}) must survive into the raster \
                     composite — the story page fill runs BEFORE the plate, not over it"
                );
                checked += 1;
            }
        }
        assert!(checked > 500, "honor={honor}: sampled {checked} plate pixels — the plate is really there");
        // Raster ships one image, so nothing may be left transparent.
        assert!(
            canvas.pixels().all(|p| p.0[3] == 255),
            "honor={honor}: the raster composite is self-contained (SQ-0510)"
        );
    }
}

/// HYBRID — the default, and what the player sees. An intro screen has NO chrome
/// ring, so hybrid must recognise the window-0 plate as a full-screen picture
/// takeover and fall through to the raster composite (SQ-0570). Before the fix
/// `picture_takeover` scanned chrome only, found nothing painted, and opened a
/// pane-wide transcript viewport over art it then never uploaded.
#[test]
fn arthur_hybrid_takes_the_picture_takeover_path() {
    for honor in [true, false] {
        let Some((_plates, session)) = arthur_intro_plates() else { return };
        let model = session.screen();
        let state = render_state(app::config::V6RenderMode::Hybrid, honor);
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
        assert_eq!(
            path, "raster",
            "honor={honor}: an intro plate fills the screen with no ring to draw, so hybrid must \
             fall through to the raster composite rather than open a transcript viewport over it"
        );
    }
}

// ── SQ-0707: the plate owns its screen — no scrollback is painted over it ────

/// Drive the intro the way the APP does — accumulating the transcript turn by
/// turn — and stop on the graveyard→Merlin plate. Returns the session parked on
/// that frame plus the `AppState` carrying the narration that preceded it.
///
/// This is the difference that let SQ-0707 through: the older cases above render
/// against a default `AppState` whose transcript is EMPTY, so nothing could be
/// painted over the art and the composite looked correct. The bug only exists
/// once there is scrollback to paint.
fn arthur_plate_frame_with_scrollback(
    mode: app::config::V6RenderMode,
    honor: bool,
) -> Option<(GameSession, app::state::AppState)> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, honor, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut state = render_state(mode, honor);
    // Tap forward until a window-0 plate is up AND real narration is behind it.
    // Frame order: title plate → restore prompt → graveyard narration → the
    // graveyard+Merlin plate. The restore prompt loops on a bare Enter, so it is
    // answered explicitly, exactly as `arthur_intro_plates` does.
    for _ in 0..8 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        state.push_transcript_kind(&r.transcript, app::state::TranscriptKind::Story);
        if r.transcript.to_lowercase().contains("y or n") {
            let r2 = session.submit_char(b'n');
            state.push_transcript_kind(&r2.transcript, app::state::TranscriptKind::Story);
        }
        let narrated = state.transcript.iter().any(|l| l.contains("shivering in the cold"));
        if narrated && session.pictures_canvas.contains_key(&0) {
            return Some((session, state));
        }
    }
    panic!("Arthur's intro should reach a plate frame with the graveyard narration behind it");
}

/// The plate reaches the composite PIXEL-FOR-PIXEL, with a full transcript
/// behind it. Every painted pixel of the plate must equal the artwork — not one
/// glyph of the previous screen's narration may land on it.
///
/// Before the fix this failed on the very first plate pixel a letter covered:
/// the graveyard→Merlin frame came back with the whole "You are shivering in the
/// cold night air…" block rasterized across Merlin.
///
/// Both pixel modes and both `honor_game_colours` modes are pinned: the plate is
/// the game's geometry, not its palette, so the artwork must survive identically
/// under all four combinations.
#[test]
fn arthur_plate_is_never_overpainted_by_scrollback_prose() {
    for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
        for honor in [true, false] {
            let Some((session, state)) = arthur_plate_frame_with_scrollback(mode, honor) else {
                return;
            };
            assert!(
                state.transcript.iter().any(|l| l.contains("shivering in the cold")),
                "harness sanity: there IS scrollback that could be painted over the art"
            );
            let model = session.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            let plate = layout.story_gfx.expect("a plate is up").clone();
            let WinNode::Graphics(gw) = &plate.node else { panic!("story_gfx is a Graphics leaf") };

            let (canvas, metrics) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &state);

            let (px0, py0, pw, ph) = PLATE;
            let mut checked = 0usize;
            for y in py0..py0 + ph {
                for x in px0..px0 + pw {
                    let src = gw.canvas.get_pixel(x, y).0;
                    if src[3] == 0 {
                        continue;
                    }
                    assert_eq!(
                        canvas.get_pixel(x, y).0,
                        src,
                        "{mode:?}/honor={honor}: ({x},{y}) inside the plate differs from the \
                         artwork — the story window's scrollback was rasterized over the \
                         illustration. Arthur erases the screen before it narrates, so a plate \
                         frame carries no prose at all (SQ-0707)"
                    );
                    checked += 1;
                }
            }
            assert!(checked > 200_000, "{mode:?}/honor={honor}: sampled {checked} plate pixels");
            assert!(
                metrics.is_none(),
                "{mode:?}/honor={honor}: the plate owns the screen, so there is no story text box \
                 to report scroll metrics for (matches the full-window-picture case, SQ-0578)"
            );
            // Raster ships one image, so nothing may be left transparent (SQ-0510).
            assert!(
                canvas.pixels().all(|p| p.0[3] == 255),
                "{mode:?}/honor={honor}: the composite is self-contained even on the early-out"
            );
        }
    }
}

/// HYBRID, end to end and across pane sizes: a plate frame reaches the terminal
/// as pure artwork — the takeover fall-through holds at every pane geometry, so
/// the plate is never composited into a cell-based transcript viewport with the
/// scrollback wrapped around it.
///
/// Swept over several pane sizes because the hybrid/raster decision is driven by
/// cell-quantized geometry and can flip between them. NOTE this is not the
/// SQ-0707 guard: on the takeover path the prose is rasterized INTO the uploaded
/// image, so it never appears as terminal letter cells either way (the original
/// bug report measured 0 letter cells and 2718 halfblock cells). The overpaint
/// itself is pinned per-pixel, in both modes, by the test above.
#[test]
fn arthur_hybrid_plate_frame_draws_no_text_cells_at_any_pane_size() {
    for honor in [true, false] {
        let Some((session, state)) =
            arthur_plate_frame_with_scrollback(app::config::V6RenderMode::Hybrid, honor)
        else {
            return;
        };
        let model = session.screen();
        for (w, h) in [(80u16, 24u16), (100, 30), (120, 40), (160, 50), (200, 60)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
            assert_eq!(
                path, "raster",
                "honor={honor} {w}x{h}: a full-screen plate has no ring to draw, so hybrid falls \
                 through to the raster composite"
            );
            let letters = buf
                .content()
                .iter()
                .filter(|c| c.symbol().chars().next().is_some_and(|ch| ch.is_alphabetic()))
                .count();
            assert_eq!(
                letters, 0,
                "honor={honor} {w}x{h}: the plate frame must be pure artwork — {letters} letter \
                 cells means the narration is being drawn on the illustration (SQ-0707)"
            );
        }
    }
}

// ── Regression: genuine inline floats are untouched ───────────────────────────

/// Zork Zero's drop-caps and room icons are drawn ON window 0's text cursor, so
/// they stay transcript-anchored inline floats: window 0 must never gain a canvas
/// and `story_gfx` must stay empty. (The whole point of the `at_cursor`
/// discriminator — this is the case the old unconditional divert got right.)
#[test]
fn zork0_window0_drop_caps_stay_inline_floats() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("Zork0 (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut saw_float = false;
    for _ in 0..8 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
        saw_float |= r
            .transcript_elems
            .iter()
            .any(|e| matches!(e, app::session::TranscriptElem::Image(_)));
        assert!(
            !session.pictures_canvas.contains_key(&0),
            "a cursor-anchored window-0 picture must never take a window canvas"
        );
        let model = session.screen();
        if let WinNode::Layered(items) = &model.root {
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            assert!(
                layout.story_gfx.is_none(),
                "Zork Zero publishes no story_gfx — its window-0 art is inline, not placed"
            );
        }
    }
    assert!(
        saw_float,
        "Zork Zero really does float window-0 pictures into the transcript (harness sanity)"
    );
}
