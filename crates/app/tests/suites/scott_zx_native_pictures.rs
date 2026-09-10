//! SQ-1480: the ZX Spectrum *Mysterious Adventures* releases' own room
//! artwork — the SAME §8.2 family-B display lists the Commodore 64 releases
//! store, decoded straight off the `.z80` snapshot rather than a PRG/D64
//! image — must reach the SAME picture band through the SAME backend-neutral
//! renderer, with the platform's palette as the only thing that differs
//! (`crate::graphics::ScottFamilyBPlatform`, `crates/app/src/graphics.rs`).
//!
//! `stories/scott-dialects/spectrum/m1goldba.z80` and
//! `stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg` are *The Golden
//! Baton* on the two platforms — the side-by-side oracle
//! `scott_c64_native_pictures.rs` already uses for the C64-vs-Blorb
//! comparison, extended here to the C64-vs-ZX one: the SAME title, the SAME
//! §8.2 canvas and geometry either way, so the two must place IDENTICALLY —
//! not merely to the same shape, since nothing about the fit differs between
//! them. Both are gitignored commercial fixtures; every case here skips
//! vacuously without them.

use app::engine::{Engine, GraphicsWindow, WinNode};
use app::render::graphics::{kitty_picker, GraphicsRender};
use app::scott_session::ScottSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui_image::picker::Picker;

use crate::fixture_paths::fixture_path;

fn goldba_z80() -> Option<Vec<u8>> {
    let path = fixture_path("scott-dialects/spectrum/m1goldba.z80");
    std::fs::read(&path).ok()
}

fn baton_prg() -> Option<Vec<u8>> {
    let path = fixture_path("scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg");
    std::fs::read(&path).ok()
}

fn zx_session() -> Option<ScottSession> {
    Some(ScottSession::new(goldba_z80()?, None).expect("m1goldba.z80 boots"))
}

fn c64_session() -> Option<ScottSession> {
    Some(ScottSession::new(baton_prg()?, None).expect("BATON.prg boots"))
}

fn both_sessions() -> Option<(ScottSession, ScottSession)> {
    match (zx_session(), c64_session()) {
        (Some(z), Some(c)) => Some((z, c)),
        _ => {
            eprintln!(
                "SKIP: needs both stories/scott-dialects/spectrum/m1goldba.z80 \
                 and stories/scott-dialects/c64/prg/MYSTADV1.D64/BATON.prg \
                 (gitignored commercial fixtures)"
            );
            None
        }
    }
}

fn picture_band(model: &app::engine::ScreenModel) -> Option<&GraphicsWindow> {
    match &model.root {
        WinNode::Pair { first, .. } => match &**first {
            WinNode::Graphics(gw) => Some(gw),
            _ => None,
        },
        _ => None,
    }
}

/// The bounding rect of every cell in `area` the render actually touched.
fn touched_rect(buf: &Buffer, area: Rect, letterbox_bg: Color) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (None, None, None, None);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell((x, y)) else { continue };
            if cell.symbol() == " " && cell.bg == letterbox_bg {
                continue;
            }
            x0 = Some(x0.map_or(x, |v: u16| v.min(x)));
            x1 = Some(x1.map_or(x, |v: u16| v.max(x)));
            y0 = Some(y0.map_or(y, |v: u16| v.min(y)));
            y1 = Some(y1.map_or(y, |v: u16| v.max(y)));
        }
    }
    match (x0, x1, y0, y1) {
        (Some(x0), Some(x1), Some(y0), Some(y1)) => {
            Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
        }
        _ => None,
    }
}

fn render_band(picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style) -> Buffer {
    let mut buf = Buffer::empty(area);
    let mut gr = GraphicsRender::default();
    gr.render(picker, gw, area, letterbox, &mut buf);
    buf
}

/// Room 1's band places IDENTICALLY on the ZX and the C64 release of the same
/// title — pinned to the exact rect (not merely compared to each other),
/// mirroring `scott_saga_pictures.rs`'s `(0, 3, 30, 9)`-shaped pin: the
/// family-B canvas is 255x94 on both platforms (2.71:1), so 30 cells of
/// width under half-blocks is 30/2.71 = 11.06 half-block pixels tall, 6
/// cells, centred at row (16 - 6) / 2 = 5.
#[test]
fn zx_and_c64_room1_bands_place_identically() {
    let Some((zx, c64)) = both_sessions() else { return };
    let zx_model = zx.screen();
    let c64_model = c64.screen();
    let zx_gw = picture_band(&zx_model).expect("ZX room 1 shows a band");
    let c64_gw = picture_band(&c64_model).expect("C64 room 1 shows a band");

    let area = Rect::new(0, 0, 30, 16);
    let letterbox_color = Color::Rgb(0, 0, 0);
    let letterbox = Style::default().bg(letterbox_color);

    for picker in [Picker::halfblocks(), kitty_picker(10, 20)] {
        let zx_rect = touched_rect(&render_band(&picker, zx_gw, area, letterbox), area, letterbox_color)
            .expect("the ZX band draws something");
        let c64_rect =
            touched_rect(&render_band(&picker, c64_gw, area, letterbox), area, letterbox_color)
                .expect("the C64 band draws something");
        assert_eq!(
            zx_rect, c64_rect,
            "the same title's picture band must place identically on both platforms"
        );
    }

    // Pinned rather than merely compared, the same way `scott_saga_pictures.rs`
    // pins its own family's rect — a decode that changed the canvas shape on
    // either platform would land somewhere else, and the point of the pin is
    // to say where.
    let got = touched_rect(
        &render_band(&Picker::halfblocks(), zx_gw, area, letterbox),
        area,
        letterbox_color,
    )
    .unwrap();
    assert_eq!(got, Rect::new(0, 5, 30, 6), "the family-B 255x94 canvas fitted into the band");
}

/// `/dump-windows` names the platform for each — the SAME `source=native
/// <platform> x<scale>` shape, differing only in the word (SQ-1463, SQ-1480).
#[test]
fn zx_and_c64_window_dumps_name_their_own_platform() {
    let Some((zx, c64)) = both_sessions() else { return };
    let zx_dump = zx.window_dump().join("\n");
    let c64_dump = c64.window_dump().join("\n");
    assert!(
        zx_dump.contains("source=native ZX Spectrum x3"),
        "ZX dump should name its own platform:\n{zx_dump}"
    );
    assert!(
        c64_dump.contains("source=native C64 x3"),
        "C64 dump should name its own platform:\n{c64_dump}"
    );
}
