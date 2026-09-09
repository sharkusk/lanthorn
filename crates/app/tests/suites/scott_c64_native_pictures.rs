//! SQ-1463: the Commodore 64 Mysterious Adventures' own room pictures, decoded
//! straight off the release's PRG/D64 image (`scott::c64::decode_family_b_pictures`,
//! SQ-1414) rather than a Blorb, must reach the SAME picture band the `.blb`
//! releases already draw through — same rows reserved above the room panel, same
//! `GraphicsWindow` shape, drawn by the same backend-neutral renderer
//! (`render::graphics::GraphicsRender::render`) with no protocol special-casing
//! anywhere in the native path.
//!
//! `stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg` (native, no Blorb) and
//! `stories/golden_baton.blb` (the SAME game, Blorb-carried pictures) are the
//! side-by-side oracle: two different releases' artwork for one title, so
//! agreement on the BAND GEOMETRY (never on pixel content — the two archives draw
//! the room at different native resolutions) is what a shared render path
//! actually promises. Both are gitignored commercial fixtures; every case here
//! skips vacuously without them.

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::render::graphics::{kitty_picker, GraphicsRender};
use app::scott_session::ScottSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui_image::picker::Picker;

use crate::fixture_paths::fixture_path;

fn baton_prg() -> Option<Vec<u8>> {
    let path = fixture_path("scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg");
    std::fs::read(&path).ok()
}

fn golden_baton_blb() -> Option<ScottSession> {
    let path = fixture_path("golden_baton.blb");
    let raw = std::fs::read(&path).ok()?;
    let blorb = blorb::Blorb::parse(raw.clone()).expect("golden_baton.blb is a valid blorb");
    let app::hints::LoadedStory::Scott(bytes) =
        app::hints::extract_story(raw).expect("extract")
    else {
        panic!("golden_baton.blb holds a Scott Adams story");
    };
    Some(ScottSession::new(bytes, Some(blorb)).expect("golden_baton.blb boots"))
}

fn native_baton() -> Option<ScottSession> {
    let bytes = baton_prg()?;
    Some(ScottSession::new(bytes, None).expect("BATON.prg boots"))
}

fn picture_band(model: &app::engine::ScreenModel) -> Option<&GraphicsWindow> {
    match &model.root {
        WinNode::Pair { first, split, .. } => match &**first {
            WinNode::Graphics(gw) => {
                let _ = split; // asserted by callers, kept here only to name the match arm
                Some(gw)
            }
            _ => None,
        },
        _ => None,
    }
}

fn reserved_rows(model: &app::engine::ScreenModel) -> Option<u16> {
    match &model.root {
        WinNode::Pair { split, first, .. } if matches!(**first, WinNode::Graphics(_)) => {
            Some(split.fixed)
        }
        _ => None,
    }
}

/// Both sources' room-1 screens, or a vacuous skip explanation. `honor_game_colours`
/// plays no part in this room-picture band (it governs TEXT-cell colour resolution
/// in `render::upper_window`, and the picture band is a raw RGBA canvas placed by
/// `GraphicsRender::render` with a caller-supplied letterbox `Style` that does not
/// consult it either) — CLAUDE.md's "pin both modes" rule exists for regressions
/// in that text-colour path, so this suite still runs the check with the config set
/// both ways to document, rather than assume, that it has no effect here.
fn both_sessions() -> Option<(ScottSession, ScottSession)> {
    let native = native_baton();
    let blorbed = golden_baton_blb();
    match (native, blorbed) {
        (Some(n), Some(b)) => Some((n, b)),
        _ => {
            eprintln!(
                "SKIP: needs both stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg \
                 and stories/golden_baton.blb (gitignored commercial fixtures)"
            );
            None
        }
    }
}

#[test]
fn native_and_blorb_room1_bands_reserve_the_same_rows_and_upscale() {
    for honor_game_colours in [true, false] {
        let Some((native, blorbed)) = both_sessions() else { return };
        let _ = honor_game_colours; // see `both_sessions`'s doc: asserted for the record only

        assert_eq!(native.current_location().unwrap().number, 1);
        assert_eq!(blorbed.current_location().unwrap().number, 1);

        let native_model = native.screen();
        let blorb_model = blorbed.screen();
        let native_rows = reserved_rows(&native_model).expect("native room 1 shows a band");
        let blorb_rows = reserved_rows(&blorb_model).expect("blorb room 1 shows a band");
        assert_eq!(
            native_rows, blorb_rows,
            "the picture band reserves the SAME rows regardless of source \
             (both come from scott_session.rs's one PICTURE_ROWS constant)"
        );

        let native_gw = picture_band(&native_model).unwrap();
        let blorb_gw = picture_band(&blorb_model).unwrap();
        assert!(native_gw.upscale, "native source stretches into the band");
        assert!(blorb_gw.upscale, "blorb source stretches into the band");
        assert_eq!(native_gw.win, blorb_gw.win, "both occupy the same window slot");
    }
}

/// The bounding rect of every cell in `area` that the render actually touched —
/// i.e. differs from the plain `letterbox` fill `GraphicsRender::render` paints
/// first. `None` if nothing was drawn.
fn touched_rect(buf: &Buffer, area: Rect, letterbox_bg: Color) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (None, None, None, None);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell((x, y)) else { continue };
            let untouched = cell.symbol() == " " && cell.bg == letterbox_bg;
            if untouched {
                continue;
            }
            x0 = Some(x0.map_or(x, |v: u16| v.min(x)));
            x1 = Some(x1.map_or(x, |v: u16| v.max(x)));
            y0 = Some(y0.map_or(y, |v: u16| v.min(y)));
            y1 = Some(y1.map_or(y, |v: u16| v.max(y)));
        }
    }
    match (x0, x1, y0, y1) {
        (Some(x0), Some(x1), Some(y0), Some(y1)) => Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)),
        _ => None,
    }
}

/// Render `gw`'s canvas through the SAME `GraphicsRender::render` the app draws
/// every graphics window with — no protocol special-casing for the native Scott
/// C64 source anywhere in `scott_session.rs` or `graphics.rs`, so this is
/// verifying the shared path, not a native-only branch.
fn render_band(picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style) -> Buffer {
    let mut buf = Buffer::empty(area);
    let mut gr = GraphicsRender::default();
    gr.render(picker, gw, area, letterbox, &mut buf);
    buf
}

#[test]
fn native_and_blorb_room1_bands_place_identically_under_halfblocks_and_kitty() {
    let Some((native, blorbed)) = both_sessions() else { return };
    let native_model = native.screen();
    let blorb_model = blorbed.screen();
    let native_gw = picture_band(&native_model).expect("native band");
    let blorb_gw = picture_band(&blorb_model).expect("blorb band");

    // A tall, narrow pane: both sources' pictures are far wider than tall (the
    // native 255x94 canvas and the blorb's own 256x96 Pict alike), so on THIS
    // shape the width is what binds the aspect-preserving fit for both — the
    // axis the fit does not saturate is what would expose a per-source drift.
    let area = Rect::new(0, 0, 30, 16);
    let letterbox_color = Color::Rgb(0, 0, 0);
    let letterbox = Style::default().bg(letterbox_color);

    // Half-blocks: `GraphicsRender::render` resamples the canvas and places a
    // `dest` rect centered in `area`, sized by `fitted_protocol`'s aspect-fit —
    // the one place a per-source difference (native 255x94 vs the Blorb's own
    // 256x96 Pict) could leak into where the image lands.
    let hb_picker = Picker::halfblocks();
    let native_hb = touched_rect(&render_band(&hb_picker, native_gw, area, letterbox), area, letterbox_color)
        .expect("native band draws something under half-blocks");
    let blorb_hb = touched_rect(&render_band(&hb_picker, blorb_gw, area, letterbox), area, letterbox_color)
        .expect("blorb band draws something under half-blocks");
    assert_eq!(
        native_hb, blorb_hb,
        "the two sources' differently-sized art still lands on the identical \
         cell rect once fitted into the same band (half-blocks)"
    );

    // Kitty: `render_kitty_virtual` places an explicit r×c grid covering the
    // WHOLE window rect regardless of canvas size (SQ-0520) — so under kitty
    // both sources trivially cover exactly `area`, which is itself the
    // guarantee this asserts: neither source's canvas causes kitty's virtual
    // placement to shrink, skip, or misplace.
    let kitty = kitty_picker(10, 20);
    let native_kitty = touched_rect(&render_band(&kitty, native_gw, area, letterbox), area, letterbox_color)
        .expect("native band places under kitty");
    let blorb_kitty = touched_rect(&render_band(&kitty, blorb_gw, area, letterbox), area, letterbox_color)
        .expect("blorb band places under kitty");
    assert_eq!(native_kitty, blorb_kitty, "same placement rect under kitty");
    assert_eq!(native_kitty, area, "kitty's explicit r×c grid covers the whole window");
}

#[test]
fn native_room1_picture_is_not_a_flat_fill() {
    let Some(native) = native_baton() else {
        eprintln!("SKIP: no stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg");
        return;
    };
    let model = native.screen();
    let gw = picture_band(&model).expect("room 1 shows a picture band");
    let distinct: std::collections::HashSet<[u8; 4]> = gw.canvas.pixels().map(|p| p.0).collect();
    assert!(
        distinct.len() > 1,
        "Golden Baton's opening forest is trees on a green strip, not a flat \
         fill — a decode collapsed to one colour ({distinct:?})"
    );
}

/// One decoded [`scott::c64::Picture`] flattened to the same `[r, g, b, 255]`
/// pixel shape `PictSource`'s canvas exposes, for a pixel-for-pixel oracle
/// comparison independent of `app::graphics`'s own conversion code.
fn picture_pixels(pic: &scott::c64::Picture) -> Vec<[u8; 4]> {
    (0..pic.height)
        .flat_map(|y| (0..pic.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let (r, g, b) = pic.rgb(x, y).expect("in-bounds");
            [r, g, b, 255]
        })
        .collect()
}

/// Falsification (CLAUDE.md "Falsify fixes"): room 1's band must be
/// `pictures[0]` under the decoder's own "room n -> image n-1" identity
/// (`scott::c64::decode_family_b_pictures`'s doc, §8.6) — decoded here
/// independently of `PictSource::get`'s `resnum.checked_sub(1)` offset, as the
/// oracle that offset is asserted against, not merely re-derived from it.
///
/// Reverting that offset to a bare `resnum` (feeding room 1 the WRONG room's
/// picture — `pictures[1]`, one past what §8.6 says room 1 shows) was checked
/// by hand: with the `- 1` removed, `got == expected_room1` below fails and
/// `got == pictures[1]`'s flattened pixels instead — this test is what would
/// have caught SQ-1463 shipping that off-by-one.
#[test]
fn native_room1_canvas_matches_the_decoder_oracle_not_an_off_by_one_room() {
    let Some(bytes) = baton_prg() else {
        eprintln!("SKIP: no stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg");
        return;
    };
    let (image, at) = scott::c64::prg_image(&bytes).expect("a PRG has a load address");
    let pictures = scott::c64::decode_family_b_pictures(image, at).expect("BATON.prg decodes");
    assert!(pictures.len() > 1, "need at least two pictures for this oracle to mean anything");

    let native = native_baton().expect("BATON.prg boots (just parsed above)");
    let model = native.screen();
    let gw = picture_band(&model).expect("room 1 shows a picture band");
    let got: Vec<[u8; 4]> = gw.canvas.pixels().map(|p| p.0).collect();

    assert_eq!(
        got,
        picture_pixels(&pictures[0]),
        "room 1 must show pictures[0] (§8.6's identity mapping), pixel for pixel"
    );
    assert_ne!(
        got,
        picture_pixels(&pictures[1]),
        "and must NOT show pictures[1] — the off-by-one this oracle guards against"
    );
}
