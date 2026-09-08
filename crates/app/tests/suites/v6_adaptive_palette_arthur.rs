//! Arthur (Infocom v6): a palette change must recolour adaptive pictures that are
//! ALREADY on screen (Blorb spec §11.3, SQ-0567).
//!
//! Arthur's decorative frame — the border art the location/date header sits in — is
//! its three `APal` adaptive pictures (54, 170, 171), drawn ONCE during the intro
//! and never again. An adaptive picture is plotted with the "Current Palette" (the
//! palette of the most recently drawn NON-adaptive picture), so when a scene
//! establishes a new one the frame is meant to follow it. Each scene carries its
//! own 16-colour PLTE:
//!
//! - churchyard — Pict 4, blue-dominant
//! - church     — Pict 10 (`ddaa88`, `775500`), brown
//! - hiding behind the gravestone — Pict 7 (`7080f0`), a different blue
//!
//! lanthorn re-decoded an adaptive picture with the Current Palette only when the
//! game DREW it, so Arthur's frame kept the churchyard palette for the whole game:
//! it stayed blue in the brown church and never shifted when the gravestone scene
//! swapped the blues.
//!
//! Asserted on the window canvases, upstream of any rendering, and run in BOTH
//! `honor_game_colours` modes — the palette here comes from picture data, not from
//! the theme, and pinning both modes is what proves that independence.
//!
//! Skips cleanly when the gitignored story is absent (CI).

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

/// Boot Arthur past the sword-in-the-stone intro to the churchyard, where the
/// frame is drawn and the game is taking commands. `None` when the story is absent.
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

/// Mean RGB of the frame window's opaque pixels, plus how many there are — the
/// colour of the band the header sits in.
fn frame_tint(session: &GameSession) -> ([u32; 3], u64) {
    let canvas = session.pictures_canvas.get(&7).expect("Arthur's frame is window 7");
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in canvas.img.pixels() {
        if p.0[3] != 0 {
            r += p.0[0] as u64;
            g += p.0[1] as u64;
            b += p.0[2] as u64;
            n += 1;
        }
    }
    let d = n.max(1);
    ([(r / d) as u32, (g / d) as u32, (b / d) as u32], n)
}

fn step(session: &mut GameSession, cmd: &str) -> String {
    let r = session.submit(cmd);
    r.transcript.trim().lines().next().unwrap_or("").trim().to_string()
}

#[test]
fn arthur_frame_follows_the_scene_palette() {
    for honor_game_colours in [true, false] {
        let Some(mut session) = arthur_at_churchyard(honor_game_colours) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let label = format!("honor_game_colours={honor_game_colours}");

        let (yard, yard_px) = frame_tint(&session);
        assert!(
            yard[2] > yard[0] && yard[2] > yard[1],
            "{label}: the churchyard frame is blue-dominant, got rgb{yard:?}"
        );

        // Into the church: its brown palette must recolour the frame drawn at boot.
        assert!(step(&mut session, "in").contains("CHURCH"), "{label}: entered the church");
        let (church, church_px) = frame_tint(&session);
        assert!(
            church[0] > church[1] && church[1] > church[2],
            "{label}: the church frame is brown (r > g > b), got rgb{church:?}"
        );
        assert_eq!(
            church_px, yard_px,
            "{label}: recoloured in place — the same pixels, not a moved or resized frame"
        );

        // Back out: the churchyard palette returns, exactly.
        assert!(step(&mut session, "west").contains("CHURCHYARD"), "{label}: back outside");
        assert_eq!(frame_tint(&session).0, yard, "{label}: the churchyard tint returns exactly");

        // Hiding swaps to a DIFFERENT blue palette — still blue, but not the same.
        let hid = step(&mut session, "hide behind gravestone");
        assert!(hid.contains("gravestone"), "{label}: hid behind the gravestone, got {hid:?}");
        let (hiding, _) = frame_tint(&session);
        assert!(
            hiding[2] > hiding[0] && hiding[2] > hiding[1],
            "{label}: still blue-dominant while hiding, got rgb{hiding:?}"
        );
        assert_ne!(
            hiding, yard,
            "{label}: but a different blue — the gravestone scene swaps the palette"
        );
    }
}

/// Composite the v6 chrome exactly as the hybrid and raster renderers do, so a test
/// sees the pixels a player sees.
fn chrome(session: &mut GameSession) -> image::RgbaImage {
    use image::Rgba;
    let model = session.screen();
    let WinNode::Layered(items) = model.root.clone() else { panic!("v6 stories build a Layered root") };
    let native = app::render::v6_layout::native_extent(&items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(&items, zvm::screen::V6Cell::DEFAULT);
    app::render::v6_layout::build_chrome_canvas(
        &layout.chrome,
        native,
        Rgba([200, 200, 200, 255]),
        Rgba([0, 0, 0, 255]),
        &app::colors::ColorScheme::terminal_default(),
        app::render::v6_layout::TextLayer::All,
        &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
    )
}

fn fkey(session: &mut GameSession, code: u8) {
    if matches!(session.pending_input(), InputKind::Line) {
        session.submit_line_with_terminator("", code);
    } else {
        session.submit_char(code);
    }
}

/// Fraction of the graphics band taken by its single most common colour — a cheap
/// read on whether the band shows the frame's dense ornament or the map's flat
/// parchment, without pinning individual pixels.
fn band_flatness(img: &image::RgbaImage) -> f64 {
    let mut counts: std::collections::HashMap<[u8; 4], u64> = std::collections::HashMap::new();
    let mut total = 0u64;
    for y in 0..192u32 {
        for x in 0..640u32 {
            *counts.entry(img.get_pixel(x, y).0).or_default() += 1;
            total += 1;
        }
    }
    counts.values().copied().max().unwrap_or(0) as f64 / total.max(1) as f64
}

/// SQ-0567: recolouring for a new palette must not disturb what covers what.
///
/// Arthur's F2 map screen draws a full-screen parchment background into window 7 —
/// the same window its frame lives in — and that background is a BASE picture, so it
/// changes the palette. A replay that simply re-plots the adaptive frame afterwards
/// puts the frame back on top and hides the map: the screen showed the ornate border
/// with a flat fill in the middle and no map at all. Replaying the window's draws in
/// their original order keeps the background over the frame, where the game put it.
///
/// Measured on the real story: the picture screen's band is 34% its commonest colour
/// (dense ornament), the map's is 90% (flat parchment). With the frame replotted on
/// top the map band looked like the picture screen's again.
#[test]
fn a_palette_replay_keeps_a_later_background_above_the_frame() {
    let Some(mut session) = arthur_at_churchyard(true) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let picture = band_flatness(&chrome(&mut session));
    assert!(picture < 0.6, "the picture screen's band is dense frame art, got {picture:.3}");

    fkey(&mut session, 134); // the map
    let map_img = chrome(&mut session);
    let map = band_flatness(&map_img);
    assert!(
        map > 0.8,
        "the map's parchment background covers the frame — the frame must NOT be \
         replotted over it. Band flatness {map:.3} (the picture screen reads {picture:.3})"
    );
    // ...but not a bare wash: the room marker and compass rose reach the composite.
    let colours: std::collections::HashSet<[u8; 4]> = (0..192)
        .flat_map(|y| (0..640).map(move |x| (x, y)))
        .map(|(x, y)| map_img.get_pixel(x, y).0)
        .collect();
    assert!(colours.len() > 3, "the map band carries content, got {} colours", colours.len());

    // Back to the picture screen: the dense frame art returns.
    fkey(&mut session, 133);
    let back = band_flatness(&chrome(&mut session));
    assert!(
        (back - picture).abs() < 1e-9,
        "the picture screen is restored exactly ({back:.3} vs {picture:.3})"
    );
}

// ── SQ-0881: what a replay must NOT recolour ─────────────────────────────────

/// The centre pixel of a decoded Pict, or `None` when it did not decode.
fn centre(img: Option<std::sync::Arc<image::DynamicImage>>) -> Option<[u8; 4]> {
    let rgba = img?.to_rgba8();
    Some(rgba.get_pixel(rgba.width() / 2, rgba.height() / 2).0)
}

/// A palette change must recolour the ADAPTIVE frame and nothing else — SQ-0881.
///
/// The replay above (SQ-0567) rebuilds a window by re-decoding its whole display
/// list, and it used to push every picture through the live palette, base ones
/// included. Arthur's map screen is where that shows: the game lays down the
/// parchment scroll (Pict 137), then the room box and compass rose, and on the
/// DOS MCGA archive those three carry palettes of their OWN — a machine whose DAC
/// had 256 entries did not have to share sixteen. Replayed through the last of
/// them, the scroll came back in `DEFAULT_PALETTE` wherever the borrowed table ran
/// out: entry 8 grey for the parchment, 9 and 10 for the rods. The Amiga archive
/// gives that same screen ONE palette, so it drew correctly throughout and hid
/// this for as long as MCGA was not the default rendition (SQ-0880).
///
/// Both halves are asserted here, because the fix is a narrowing and a narrowing
/// can take too much: the frame — Picts 54, 170 and 171, this archive's only
/// `APal` entries — must still follow.
///
/// Falsifiable: send base pictures through `adaptive_image` again and the scroll
/// comes back `[85, 85, 85, 255]`, the reported grey.
#[test]
fn a_replay_recolours_the_adaptive_frame_and_leaves_base_art_alone() {
    let mg1 = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/arthur.mg1");
    let Ok(bytes) = std::fs::read(&mg1) else {
        eprintln!("SKIP: gitignored archive missing at {}", mg1.display());
        return;
    };
    let pics = blorb::infocom_pics::InfocomPics::parse(bytes).expect("arthur.mg1 parses");
    assert_eq!(pics.adaptive_pictures(), vec![54, 170, 171], "the archive's APal set");
    let mut src = PictSource::from_native(pics);

    // The map's parchment scroll, drawn through the palette it carries.
    let scroll = centre(src.image(137)).expect("Pict 137 decodes");
    assert_eq!(scroll, [255, 192, 133, 255], "the scroll's own parchment");

    // Pict 115 — a marker the game draws onto that scroll a moment later —
    // carries a DIFFERENT palette, and drawing it establishes it as Current.
    let before = src.current_palette().map(<[u8]>::to_vec);
    let _ = src.image(115);
    assert_ne!(src.current_palette().map(<[u8]>::to_vec), before, "115 loads its own palette");

    // The replay hands the scroll back unchanged…
    assert_eq!(
        centre(src.image_under_current_palette(137)),
        Some(scroll),
        "a base picture is replayed in ITS OWN colours, not the last picture's"
    );
    // …and the frame still follows the palette, which is what the replay is for.
    let frame_now = centre(src.image_under_current_palette(170)).expect("Pict 170 decodes");
    src.set_current_palette(before);
    assert_ne!(
        centre(src.image_under_current_palette(170)),
        Some(frame_now),
        "an adaptive picture still tracks the Current Palette"
    );
}
