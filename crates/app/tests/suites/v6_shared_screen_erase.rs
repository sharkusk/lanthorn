//! v6 windows are clipping regions over ONE shared screen, not independent drawing
//! surfaces (ZMSD §8, SQ-0568).
//!
//! The standard is explicit: windows "usually lie on top of each other", plotting is
//! "clipped to the current window, and anything showing through is plotted onto the
//! screen", and "subsequent movements of the window do not move what was printed".
//! A plotted pixel belongs to the SCREEN — so erasing a region must remove whatever
//! any window put there, not only what the erased window drew itself.
//!
//! lanthorn keeps a canvas per window, and an erase used to touch only that window's
//! own canvas. Arthur shows what that costs: its F2 map screen paints a full-screen
//! background into window 7, and switching back to the F1 picture screen erases
//! windows 2, 5 and 6 — never 7 — so the map background stayed under every later
//! screen for the rest of the game, and the picture insert came back over the top of
//! it.
//!
//! Run in BOTH `honor_game_colours` modes: these assertions count opaque pixels
//! rather than read colours, so the mode must make no difference — which is worth
//! pinning rather than assuming.
//!
//! Skips cleanly when the gitignored story is absent (CI).

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

/// Arthur's window 2 (the illustration panel) in screen unit pixels, from its own
/// window table: origin (28, 0), 584×192.
const WIN2_RECT: (u32, u32, u32, u32) = (28, 0, 584, 192);

/// Boot Arthur past the intro to the churchyard, showing the F1 picture screen.
fn arthur_at_churchyard(honor_game_colours: bool) -> Option<GameSession> {
    let story_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session = GameSession::new_with_trace(
        story_bytes, honor_game_colours, false, None, false, picture_dims, std_window, None, None
    )
    .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    let _ = session.take_transcript();
    Some(session)
}

/// Press a function key, however the game happens to be waiting.
fn fkey(session: &mut GameSession, code: u8) {
    if matches!(session.pending_input(), InputKind::Line) {
        session.submit_line_with_terminator("", code);
    } else {
        session.submit_char(code);
    }
}

/// Opaque pixels of `win`'s canvas that lie inside a SCREEN rect.
fn opaque_inside(session: &GameSession, win: u8, rect: (u32, u32, u32, u32)) -> u64 {
    let Some(c) = session.pictures_canvas.get(&win) else { return 0 };
    let (ox, oy, w, h) = rect;
    c.img
        .enumerate_pixels()
        .filter(|(x, y, p)| p.0[3] != 0 && *x >= ox && *x < ox + w && *y >= oy && *y < oy + h)
        .count() as u64
}

/// Total opaque pixels of a window's canvas (0 when it has none).
fn opaque(session: &GameSession, win: u8) -> u64 {
    session
        .pictures_canvas
        .get(&win)
        .map(|c| c.img.pixels().filter(|p| p.0[3] != 0).count() as u64)
        .unwrap_or(0)
}

#[test]
fn switching_arthur_screens_leaves_no_residue_from_the_other() {
    for honor_game_colours in [true, false] {
        let Some(mut session) = arthur_at_churchyard(honor_game_colours) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let label = format!("honor_game_colours={honor_game_colours}");

        // The F1 picture screen: the frame in window 7, the illustration in window 2.
        let frame_baseline = opaque_inside(&session, 7, WIN2_RECT);
        let picture_baseline = opaque(&session, 2);
        assert!(frame_baseline > 0 && picture_baseline > 0, "{label}: F1 screen is drawn");
        // The picture BORDER's vertical pieces sit below the map background's band,
        // in the side-strip windows: (6, 300) and (630, 300).
        let border = |s: &GameSession, x: u32, y: u32| {
            s.pictures_canvas.get(&7).is_some_and(|c| c.img.get_pixel(x, y).0[3] != 0)
        };
        assert!(border(&session, 6, 300) && border(&session, 630, 300), "{label}: border drawn");

        // F2, the map: its background goes into window 7 and covers window 2 whole.
        fkey(&mut session, 134);
        assert_eq!(
            opaque_inside(&session, 7, WIN2_RECT),
            u64::from(WIN2_RECT.2 * WIN2_RECT.3),
            "{label}: the map background fills window 2's rect — drawn into window 7"
        );
        assert_ne!(opaque(&session, 2), picture_baseline, "{label}: the map replaced the picture");
        // The border's vertical pieces are removed by the erase of the side-strip
        // windows they sit in. This is where the palette replot (SQ-0567) used to put
        // them straight back — the map screen then kept the picture border over it.
        assert!(!border(&session, 6, 300), "{label}: left border cleared on the map");
        assert!(!border(&session, 630, 300), "{label}: right border cleared on the map");

        // Back to F1. The game erases windows 2/5/6 and never 7, so the map
        // background can only go if an erase clears the shared screen region.
        fkey(&mut session, 133);
        assert_eq!(
            opaque_inside(&session, 7, WIN2_RECT),
            frame_baseline,
            "{label}: no map background left behind window 2"
        );
        assert_eq!(
            opaque(&session, 2),
            picture_baseline,
            "{label}: and the picture insert is repainted, exactly as it first appeared"
        );
        assert!(
            border(&session, 6, 300) && border(&session, 630, 300),
            "{label}: the border is drawn again on the picture screen"
        );

        // A second round trip must not drift — residue would accumulate per switch.
        fkey(&mut session, 134);
        fkey(&mut session, 133);
        assert_eq!(
            (opaque_inside(&session, 7, WIN2_RECT), opaque(&session, 2)),
            (frame_baseline, picture_baseline),
            "{label}: still exact after a second switch"
        );
    }
}

/// The same round trip from inside the church, where the scene also carries an inset
/// picture — the insert must come back there too, not just in the opening scene.
#[test]
fn the_church_scene_survives_a_screen_switch() {
    for honor_game_colours in [true, false] {
        let Some(mut session) = arthur_at_churchyard(honor_game_colours) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let label = format!("honor_game_colours={honor_game_colours}");
        let entered = session.submit("in");
        assert!(
            entered.transcript.contains("CHURCH"),
            "{label}: entered the church, got {:?}",
            entered.transcript.trim().lines().next().unwrap_or("")
        );
        let frame = opaque_inside(&session, 7, WIN2_RECT);
        let picture = opaque(&session, 2);

        fkey(&mut session, 134); // map
        fkey(&mut session, 133); // back to the picture
        assert_eq!(
            (opaque_inside(&session, 7, WIN2_RECT), opaque(&session, 2)),
            (frame, picture),
            "{label}: the church screen is restored exactly"
        );
    }
}
