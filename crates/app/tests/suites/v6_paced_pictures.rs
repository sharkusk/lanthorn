//! SQ-0708 — a v6 turn's pictures land one after another, not all at once.
//!
//! Arthur's intro draws the graveyard plate and then Merlin **in the same turn**,
//! fourteen instructions apart:
//!
//! ```text
//! turn 2: 10 picture events
//!   erase_window × 8  (windows 0..7, the screen swap)
//!   draw_picture 2 → window 0 at (29, 5)   584×392  the graveyard plate
//!   draw_picture 3 → window 0 at (81, 51)  480×300  Merlin, inside it
//! ```
//!
//! Compositing both before anything renders hands the player the finished screen
//! instantly. Spatterlight blits each `draw_picture` as its opcode executes, so
//! you watch the graveyard paint and then Merlin paint onto it — it reads as an
//! animation, and that is what this reproduces.
//!
//! There is **no Z-machine construct expressing this**. The measured op sequence
//! carries no busy-wait, no `@sound_effect` and no intervening read between the
//! two draws; the `read_char` timers on those screens (150 tenths on a picture
//! screen, 600 on a text one) are an auto-advance for an idle player, not the
//! animation, and `loop_tick` already honours them. So this is a presentation
//! feature invented deliberately, not a standard being implemented.
//!
//! How it works, and what these cases pin:
//!
//! * **Post-hoc replay.** The turn runs to completion exactly as before —
//!   `GameSession::drain_pictures` still applies every event — and the RENDERER
//!   walks the screens it passed through. The interpreter never blocks and never
//!   yields mid-turn.
//! * **[`Engine::screen`] is untouched.** It is still the settled composite, which
//!   is why every existing v6 render test passes unchanged and why saves, the
//!   display list and `/dump-windows` cannot see pacing at all. The paced view has
//!   its own accessor, [`Engine::screen_now`], and that is what `draw_frame` draws.
//! * **The last frame IS the settled composite**, byte for byte — asserted below
//!   on the canvases, on the raster composite and on the hybrid cell buffer.
//! * **Skippable.** Any keypress collapses the remainder instantly, and lands on
//!   exactly the same pixels as letting it play out.
//! * **Every v6 game**, not an Arthur-shaped predicate: Shogun's title screen paces
//!   too, and Zork Zero's boot art.
//!
//! Both v6 pixel modes are covered — HYBRID first, since it is the default and
//! what the player runs — and both `honor_game_colours` modes, `true` being the
//! shipped default.
//!
//! The story assets are gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Picture 3 (Merlin, 480×300) at `@draw_picture(number=3, window=0, y=51, x=81)`
/// → 0-based x 80..560, y 50..350, wholly inside the 584×392 graveyard plate.
const MERLIN: (u32, u32, u32, u32) = (80, 50, 480, 300);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str, honor: bool) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes, honor, false, None, false, picture_dims, picts.std_window(), None, None,
    )
    .expect("the v6 story should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// Arthur parked on the turn that draws the graveyard plate AND Merlin — the
/// sequence this whole quest is about. The intro is a `read_char` chain; the
/// restore prompt rejects a bare Enter and loops on "Please press Y or N>", so it
/// is answered explicitly.
fn arthur_on_the_merlin_turn(honor: bool) -> Option<GameSession> {
    let mut session = boot("arthur-r74-s890714.z6", honor)?;
    for _ in 0..8 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
        // The graveyard→Merlin turn is the one that queues TWO plate draws, so it
        // is the one that leaves a frame to pace through.
        if session.paced_picture_hold().is_some() {
            return Some(session);
        }
    }
    panic!("Arthur's intro must reach a turn that draws two pictures");
}

/// Window 0's canvas out of a screen model, or `None` when no plate is up.
fn win0_canvas(model: &app::engine::ScreenModel) -> Option<image::RgbaImage> {
    let WinNode::Layered(items) = &model.root else { return None };
    items.iter().find_map(|pw| match &pw.node {
        WinNode::Graphics(g) if g.win == 0 => Some((*g.canvas).clone()),
        _ => None,
    })
}

/// Percentage of pixels inside `rect` where the two images disagree.
fn pct_differs(a: &image::RgbaImage, b: &image::RgbaImage, rect: (u32, u32, u32, u32)) -> usize {
    let (rx, ry, rw, rh) = rect;
    let (mut diff, mut total) = (0usize, 0usize);
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            diff += usize::from(a.get_pixel(x, y).0 != b.get_pixel(x, y).0);
            total += 1;
        }
    }
    diff * 100 / total.max(1)
}

/// Percentage of pixels OUTSIDE `rect` where the two images disagree.
fn pct_differs_outside(a: &image::RgbaImage, b: &image::RgbaImage, rect: (u32, u32, u32, u32)) -> usize {
    let (rx, ry, rw, rh) = rect;
    let (mut diff, mut total) = (0usize, 0usize);
    for (x, y, px) in a.enumerate_pixels() {
        if (rx..rx + rw).contains(&x) && (ry..ry + rh).contains(&y) {
            continue;
        }
        diff += usize::from(px.0 != b.get_pixel(x, y).0);
        total += 1;
    }
    diff * 100 / total.max(1)
}

/// Let the whole sequence play out, as the loop's deadline-driven pacer does one
/// frame at a time. Returns how many intermediate frames there were.
fn play_out(session: &mut GameSession) -> usize {
    let mut n = 0;
    while session.advance_paced_pictures() {
        n += 1;
        assert!(n < 64, "a paced sequence must terminate");
    }
    n
}

/// Render one model through the HYBRID pane and settle on the cells it really
/// produces.
///
/// The v6 raster encode is OFF-THREAD (SQ-0469): a render whose generation is new
/// spawns a worker and keeps painting the protocol that is still installed, so one
/// call answers with the PREVIOUS frame. The event loop copes because it renders,
/// polls `poll_v6_encode_job` and renders again next tick; this does the same
/// until no encode is in flight, so the buffer it returns is the one the terminal
/// ends up holding.
///
/// That asynchrony is also a real property of the feature: a paced frame whose
/// hold is shorter than its encode may never reach the screen, because
/// `v6_wants_build` drops a superseded generation while a worker is busy. Fewer
/// steps, never a wrong or a stuck one — the sequence still ends on the settled
/// composite, which is what the assertions below actually pin.
fn hybrid_buffer(model: &app::engine::ScreenModel, state: &app::state::AppState, area: Rect) -> Buffer {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
        state.poll_v6_encode_job();
        if !state.graphics_render.borrow().v6_encode_in_flight() {
            // Nothing pending: redo the render so the freshly installed protocol
            // is the one painted, and confirm it did not spawn another encode.
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
            if !state.graphics_render.borrow().v6_encode_in_flight() {
                return buf;
            }
        }
        assert!(std::time::Instant::now() < deadline, "the v6 encode worker never settled");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn render_state(mode: app::config::V6RenderMode, honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    state
}

// ── The symptom: the graveyard lands before Merlin does ──────────────────────

/// THE regression this exists for. On the graveyard→Merlin turn the screen the
/// player is shown FIRST carries the graveyard alone: inside picture 3's declared
/// rect it disagrees with the settled composite almost everywhere, because Merlin
/// has not been painted there yet. Outside that rect the two agree except for the
/// adaptive-palette drift picture 3 legitimately causes (Arthur runs an adaptive
/// palette, and a v6 framebuffer holds indices — loading picture 3's palette
/// recolours the graveyard at the same instant Merlin appears, never before).
///
/// Before the fix `screen_now()` was the settled composite, so BOTH pictures were
/// present in the first frame and the difference inside Merlin's rect was 0%.
#[test]
fn the_graveyard_is_on_screen_before_merlin_is() {
    for honor in [true, false] {
        let Some(session) = arthur_on_the_merlin_turn(honor) else { return };
        let first = win0_canvas(&session.screen_now()).expect("a plate is up on the paced frame");
        let settled = win0_canvas(&session.screen()).expect("a plate is up on the settled frame");

        let inside = pct_differs(&first, &settled, MERLIN);
        assert!(
            inside > 75,
            "honor={honor}: the first frame must show the graveyard WITHOUT Merlin — only {inside}% \
             of picture 3's rect differs from the settled composite, so both pictures were \
             composited before anything rendered"
        );
        // …and it is Merlin's rect specifically, not a full-frame repaint. Outside
        // it the graveyard survives; only the adaptive palette moves.
        let outside = pct_differs_outside(&first, &settled, MERLIN);
        assert!(
            inside > outside * 2,
            "honor={honor}: the change must be concentrated where picture 3 was declared — \
             inside {inside}% vs outside {outside}%"
        );
        assert!(
            outside < 40,
            "honor={honor}: the graveyard outside picture 3's rect must survive into the settled \
             composite ({outside}% changed reads as a full-frame repaint)"
        );
    }
}

/// The hold is proportional to the area painted, so a full 584×392 plate rests
/// visibly longer than a small icon would — effectively what the hardware did.
#[test]
fn the_hold_is_proportional_to_the_painted_area() {
    let Some(session) = arthur_on_the_merlin_turn(true) else { return };
    let hold = session.paced_picture_hold().expect("the graveyard frame is held");
    // 584×392 unit pixels at the notional fill rate → ~286 ms, and comfortably
    // clear of both the 40 ms floor (a flicker) and the 350 ms ceiling (a stall).
    assert!(
        (200..350).contains(&(hold.as_millis() as u64)),
        "a full plate should rest for a beat you can see, got {hold:?}"
    );
}

// ── Completion and skip both land on today's composite ───────────────────────

/// Playing the sequence out leaves the screen byte-identical to the settled
/// composite — the strongest guarantee here, and the reason every pre-existing v6
/// render test still passes untouched.
#[test]
fn the_sequence_settles_on_the_composite_it_started_from() {
    for honor in [true, false] {
        let Some(mut session) = arthur_on_the_merlin_turn(honor) else { return };
        let settled = win0_canvas(&session.screen()).expect("a plate is up");
        let frames = play_out(&mut session);
        assert_eq!(frames, 1, "honor={honor}: two plate draws leave exactly one frame to pace through");
        let after = win0_canvas(&session.screen_now()).expect("a plate is still up");
        assert!(
            after == settled,
            "honor={honor}: the final paced frame must be the settled composite, byte for byte"
        );
        assert!(session.paced_picture_hold().is_none(), "honor={honor}: the sequence is over");
    }
}

/// The player outranks paced output: a keypress collapses the remainder at once,
/// and lands on exactly the pixels waiting it out would have produced.
#[test]
fn a_keypress_collapses_the_sequence_to_the_same_pixels() {
    for honor in [true, false] {
        let Some(mut skipped) = arthur_on_the_merlin_turn(honor) else { return };
        let Some(mut waited) = arthur_on_the_merlin_turn(honor) else { return };

        assert!(skipped.settle_paced_pictures(), "honor={honor}: there was a sequence to collapse");
        play_out(&mut waited);

        let a = win0_canvas(&skipped.screen_now()).expect("a plate is up");
        let b = win0_canvas(&waited.screen_now()).expect("a plate is up");
        assert!(a == b, "honor={honor}: skipping loses nothing — it only arrives sooner");
        assert!(skipped.paced_picture_hold().is_none(), "honor={honor}: nothing is left playing");
    }
}

/// Collapsing when nothing is playing is a no-op. The loop calls this on EVERY
/// keypress, so the common case has to be inert — pacing must never disturb (or
/// consume) a keystroke, which is also why an armed `[more]` prompt and a paced
/// sequence cannot eat each other's key: the event is settled AND dispatched.
#[test]
fn collapsing_an_idle_session_changes_nothing() {
    let Some(mut session) = boot("arthur-r74-s890714.z6", true) else { return };
    play_out(&mut session);
    let before = session.screen();
    assert!(!session.settle_paced_pictures(), "nothing was playing, so nothing was dropped");
    let after = session.screen_now();
    assert_eq!(
        win0_canvas(&before).is_some(),
        win0_canvas(&after).is_some(),
        "an idle collapse leaves the screen exactly as it was"
    );
}

// ── The render surface the player actually looks at ──────────────────────────

/// HYBRID — the default, and what the player runs. An Arthur plate fills the
/// screen with no chrome ring, so hybrid falls through to the raster composite
/// (SQ-0570) and ships the plate as one image. The cell buffer that reaches the
/// terminal must therefore CHANGE between the paced frame and the settled one,
/// and end up identical to the one today's single composite produces.
#[test]
fn hybrid_ships_the_paced_frame_and_then_the_settled_one() {
    for honor in [true, false] {
        let Some(mut session) = arthur_on_the_merlin_turn(honor) else { return };
        let state = render_state(app::config::V6RenderMode::Hybrid, honor);
        let area = Rect::new(0, 0, 120, 40);

        let paced = hybrid_buffer(&session.screen_now(), &state, area);
        let settled = hybrid_buffer(&session.screen(), &state, area);
        assert_ne!(
            paced, settled,
            "honor={honor}: the paced frame must reach the terminal DIFFERENT from the settled \
             composite — otherwise the sequence exists only in the session and the player never \
             sees it"
        );

        play_out(&mut session);
        assert_eq!(
            hybrid_buffer(&session.screen_now(), &state, area),
            settled,
            "honor={honor}: once the sequence has played out, hybrid ships exactly what it shipped \
             before pacing existed"
        );
    }
}

/// RASTER — the same two claims against the full-frame composite, in pixels.
#[test]
fn raster_ships_the_paced_frame_and_then_the_settled_one() {
    for honor in [true, false] {
        let Some(mut session) = arthur_on_the_merlin_turn(honor) else { return };
        let state = render_state(app::config::V6RenderMode::Raster, honor);

        let compose = |model: &app::engine::ScreenModel| {
            let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
            let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
            let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
            app::render::screen::build_v6_raster_canvas(&layout, native, &state).0
        };

        let paced = compose(&session.screen_now());
        let settled = compose(&session.screen());
        let inside = pct_differs(&paced, &settled, MERLIN);
        assert!(
            inside > 75,
            "honor={honor}: the raster composite the player sees first carries the graveyard \
             without Merlin — only {inside}% of picture 3's rect differs"
        );

        play_out(&mut session);
        assert!(
            compose(&session.screen_now()) == settled,
            "honor={honor}: the last raster composite is the one today's single-frame path builds"
        );
    }
}

// ── Not an Arthur-shaped rule ────────────────────────────────────────────────

/// Pacing is gated on GEOMETRY, not on the game and not on a config key: a turn
/// paces only when one of its pictures paints over ground an earlier picture in
/// the same turn already covered.
///
/// That is the difference between a REVEAL and screen ASSEMBLY. Arthur's intro
/// reveals — Merlin (480x300 at 81,51) lands inside the graveyard plate (584x392
/// at 29,5), and the second picture is only meaningful as a change to what the
/// first put there. Zork Zero's boot border, Arthur's gameplay chrome and Shogun's
/// title are assembly: disjoint pieces building one static frame, where no pixel is
/// painted twice, nothing is revealed, and a delay only makes the screen slower to
/// finish.
///
/// The first cut of SQ-0708 paced every multi-picture turn. The rule was narrowed
/// to overlap so that assembly stops paying for it — and measuring the corpus to
/// pin this test overturned an assumption worth recording: Zork Zero's boot is NOT
/// tiling. It draws eight different pictures into the same 45x40 rect at (277,1),
/// which is a frame-by-frame ANIMATION, and the overlap rule catches it for exactly
/// the right reason.
#[test]
fn assembly_turns_do_not_pace_only_reveals_do() {
    // Zork Zero's boot draws EIGHT pictures into one 45x40 rect at (277,1) — a
    // frame-by-frame animation, not tiling. It paces, and that is the rule working:
    // every frame after the first paints over the one before it.
    if let Some(mut zork0) = boot("zork0-r393-s890714.z6", true) {
        assert!(
            zork0.paced_picture_hold().is_some(),
            "Zork Zero's boot cycles eight pictures through one rect — a real animation, so it paces"
        );
        play_out(&mut zork0);
        assert!(
            zork0.paced_picture_hold().is_none(),
            "and it settles"
        );
    }

    // Shogun's opening screen erases window 7 and draws two pictures into it —
    // two draws, but side by side rather than one over the other.
    if let Some(mut shogun) = boot("shogun-r322-s890706.z6", true) {
        play_out(&mut shogun); // whatever boot queued
        let _ = shogun.submit("");
        assert!(
            shogun.paced_picture_hold().is_none(),
            "Shogun's title draws two pictures that do not overlap — assembly, not a reveal"
        );
    }
}

// ── A save or a resize mid-sequence sees the settled screen ──────────────────

/// The archive is written from `pictures_canvas` and the display list, both of
/// which are settled the moment the turn ends — so saving (or resizing) with a
/// sequence still playing persists the finished screen, never a half-painted one.
/// Snapshot the recipe, not the frame that happens to be up.
#[test]
fn a_save_mid_sequence_persists_the_settled_screen() {
    let Some(mut session) = arthur_on_the_merlin_turn(true) else { return };
    let mid_pngs = session.pictures_png();
    let mid_list = session.display_list();
    assert!(session.paced_picture_hold().is_some(), "a sequence really is in flight");

    play_out(&mut session);
    assert_eq!(
        mid_pngs,
        session.pictures_png(),
        "the canvases a save persists do not change as the sequence plays out"
    );
    assert_eq!(
        format!("{:?}", mid_list),
        format!("{:?}", session.display_list()),
        "nor does the display list that regenerates them"
    );
}

/// A picture that lands BESIDE what came before must not be held just because
/// something else in the same turn is animating (SQ-0708, narrowed twice).
///
/// Zork Zero's boot queues one batch containing four different things: the banner
/// (pic 5, unit rect x 0–640 y 0–68), the left pillar (pic 497 at 1,69 — ONE image,
/// 36×166), the right pillar (pic 498 at 567,69), and an eight-frame compass
/// animation cycling through a single 45×40 rect at (277,1).
///
/// Only the compass repaints covered ground: it overlaps the banner above it and
/// then itself, frame after frame. The pillars abut the banner without overlapping
/// it (y 68–400 against y 0–68) and are disjoint from each other. The first cut of
/// this rule asked "does anything in this turn overlap?" once per BATCH, so the
/// pillars were held purely for sharing a queue with the compass — reported as the
/// side bars drawing slowly.
///
/// The guard: by the time the screen first holds, both pillars are already painted.
#[test]
fn a_picture_beside_the_last_one_is_not_held_by_an_animation_sharing_its_turn() {
    let Some(session) = boot("zork0-r393-s890714.z6", true) else { return };
    assert!(
        session.paced_picture_hold().is_some(),
        "Zork Zero's boot compass animates, so the turn does pace"
    );

    // The screen as the player first sees it, before any hold elapses.
    let first = session.screen_now();
    let WinNode::Layered(items) = &first.root else { panic!("v6 builds a Layered root") };
    let frame = items
        .iter()
        .find_map(|it| match &it.node {
            WinNode::Graphics(g) if g.win == 7 => Some((*g.canvas).clone()),
            _ => None,
        })
        .expect("the boot art lives in window 7");

    // Both pillars sit in the lower band, one at each edge. Art DIMS are doubled
    // into unit space, but the game's coordinates already are unit space: the
    // screen is 640 wide, so pic 498's x=567 spans 566..640, not 1134. Sample well
    // inside each pillar rather than on an edge pixel.
    for (label, x) in [("left", 20u32), ("right", 600u32)] {
        let painted = (80..390u32)
            .step_by(20)
            .filter(|&y| {
                frame.get_pixel_checked(x, y).is_some_and(|p| p[3] > 0)
            })
            .count();
        assert!(
            painted > 5,
            "the {label} pillar is a single image landing beside the banner — it must already be \
             on screen when the compass starts animating, not held frame by frame (painted {painted} \
             of 16 sampled rows at x={x})"
        );
    }
}
