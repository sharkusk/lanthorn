//! SQ-1711 — Narcolepsy's thought bubble, on the screen a real kitty terminal resolves.
//!
//! The bubble is text window 2 (white) plus four graphics windows the game never
//! draws into (design rects 400,0 80x600 / 720,0 80x600 / 480,0 240x60 /
//! 480,438 240x162), clipped by the window mask (`narco.blorb` Pict 3: black =
//! opaque, white = transparent). Glk says an undrawn graphics window is white, so
//! every mask-visible cell of the right half must read the same as window 2.
//!
//! The cell-buffer harness (`sq1703_glk_mask`) passed while the real screen showed
//! three of the four margins DARK: a kitty upload rides as a prefix on the first
//! placeholder cell of its image, and the mask pass `reset()` that cell whenever the
//! window's top-left corner lay outside the bubble — which is the case for windows 4,
//! 6 and 8 — so the image never reached the terminal. This suite reads the pixels the
//! oracle resolves from lanthorn's own bytes, so it is the layer that sees that.
//!
//! Real specimen `narco.blorb`, skips vacuously without it. Both
//! `honor_game_colours` modes, before waking (0 inputs) and after `wake up` + a
//! key (2 inputs).

#[cfg(not(unix))]
#[test]
fn the_narcolepsy_bubble_test_is_unix_only() {
    eprintln!("SKIP: driving a real terminal needs a pty, which this platform does not have");
}

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use super::pty_stream::{self, driver, oracle};

#[cfg(unix)]
const COLS: u16 = 92;
#[cfg(unix)]
const ROWS: u16 = 37;
/// The story pane inside the app's title border: the frame the mask spans.
#[cfg(unix)]
const FRAME: (u32, u32, u32, u32) = (1, 1, 90, 34);
#[cfg(unix)]
const CELL: (u16, u16) = (8, 18);

/// The most common colour among the pixels of cell `(col, row)`.
#[cfg(unix)]
fn dominant(screen: &image::RgbaImage, col: u32, row: u32) -> [u8; 3] {
    let mut counts: std::collections::BTreeMap<[u8; 3], u32> = std::collections::BTreeMap::new();
    for y in row * u32::from(CELL.1)..(row + 1) * u32::from(CELL.1) {
        for x in col * u32::from(CELL.0)..(col + 1) * u32::from(CELL.0) {
            let p = screen.get_pixel(x, y).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c).unwrap()
}

/// Fraction of the screen cell's design-pixel box the mask leaves opaque (black),
/// or `None` for a cell outside the frame.
#[cfg(unix)]
fn opaque_fraction(mask: &image::RgbaImage, col: u32, row: u32) -> Option<f64> {
    let (fx, fy, fw, fh) = FRAME;
    if col < fx || row < fy || col >= fx + fw || row >= fy + fh {
        return None;
    }
    let (c, r) = (col - fx, row - fy);
    let (mw, mh) = (mask.width(), mask.height());
    let (x0, x1) = (c * mw / fw, ((c + 1) * mw / fw).max(c * mw / fw + 1));
    let (y0, y1) = (r * mh / fh, ((r + 1) * mh / fh).max(r * mh / fh + 1));
    let (mut black, mut all) = (0u32, 0u32);
    for y in y0..y1.min(mh) {
        for x in x0..x1.min(mw) {
            all += 1;
            if mask.get_pixel(x, y).0[0] < 128 {
                black += 1;
            }
        }
    }
    Some(f64::from(black) / f64::from(all.max(1)))
}

/// A screen cell wholly inside the bubble within a design-pixel window rect,
/// nearest the rect's centre.
#[cfg(unix)]
fn probe(mask: &image::RgbaImage, (dx, dy, dw, dh): (u32, u32, u32, u32)) -> (u16, u16) {
    let (fx, fy, fw, fh) = FRAME;
    let (cx, cy) = (dx + dw / 2, dy + dh / 2);
    let mut best: Option<(u32, u16, u16)> = None;
    for row in fy..fy + fh {
        for col in fx..fx + fw {
            let (px, py) = ((col - fx) * 800 / fw + 4, (row - fy) * 600 / fh + 8);
            if px < dx || py < dy || px >= dx + dw || py >= dy + dh {
                continue;
            }
            if opaque_fraction(mask, col, row) == Some(1.0) {
                let d = px.abs_diff(cx) + py.abs_diff(cy);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, col as u16, row as u16));
                }
            }
        }
    }
    let (_, c, r) = best.expect("a wholly-bubble cell inside the window");
    (c, r)
}

#[cfg(unix)]
fn bubble(honor: bool, wake: bool) {
    let story = driver::stories_dir().join("narco.blorb");
    if !story.is_file() {
        eprintln!("SKIP: gitignored story missing at {}", story.display());
        return;
    }
    let mask = app::graphics::PictSource::resolve(&story, None).image(3).expect("mask Pict 3 decodes").to_rgba8();
    let inputs = if wake { 2 } else { 0 };
    let user_dir = std::env::temp_dir().join(format!("lanthorn-sq1711-{}-{honor}-{wake}", std::process::id()));
    let _ = std::fs::remove_dir_all(&user_dir);
    std::fs::create_dir_all(&user_dir).expect("a throwaway lanthorn home");
    std::fs::write(user_dir.join("config.toml"), format!("honor_game_colours = {honor}\n")).expect("seeding the colour policy");

    let mut spec = driver::Spec::new(env!("CARGO_BIN_EXE_lanthorn"), &story, &user_dir);
    spec.cols = COLS;
    spec.rows = ROWS;
    spec.cell_w = CELL.0;
    spec.cell_h = CELL.1;
    spec.keys = vec![driver::Key::Wait(Duration::from_millis(5000))];
    if wake {
        spec.keys.extend([
            driver::Key::Bytes(b"wake up\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(2500)),
            driver::Key::Bytes(b"\r".to_vec()),
            driver::Key::Wait(Duration::from_millis(2500)),
        ]);
    }
    let cap = driver::run(spec).expect("the pty harness should boot lanthorn");
    let neg = cap.negotiated();
    assert!(neg.is_kitty(), "the capture must exercise the kitty path: {}", neg.explain());
    let res = oracle::resolve(
        &cap.terminal_bytes(),
        cap.spec.cols,
        cap.spec.rows,
        u32::from(cap.spec.cell_w),
        u32::from(cap.spec.cell_h),
        Some((pty_stream::ANSWERED_FG, pty_stream::ANSWERED_BG)),
    );
    let screen = pty_stream::raster::render(&res);
    let _ = std::fs::remove_dir_all(&user_dir);
    let tag = format!("honor={honor} inputs={inputs}");

    // Window 2's own cells (design 480,60 240x378 -> cells ~55..82 x 4..25) are the
    // reference: white when honoured, the theme's ground when not.
    let reference = dominant(&screen, 68, 12);
    if honor {
        assert_eq!(reference, [255, 255, 255], "{tag}: window 2 is Glk white");
    }

    // Each margin window must be on screen as an image the terminal holds, found by
    // where its cells are: left strip, right strip, top and bottom of the text box.
    let image_at = |col: u16, row: u16| res.cell(row, col).image_id;
    for (name, rect) in [
        ("window 4 (left)", (400, 0, 80, 600)),
        ("window 6 (right)", (720, 0, 80, 600)),
        ("window 8 (top)", (480, 0, 240, 60)),
        ("window 10 (bottom)", (480, 438, 240, 162)),
    ] {
        // The probe is a cell wholly inside the bubble within the window, so it must be an image.
        let (col, row) = probe(&mask, rect);
        let id = image_at(col, row);
        // Unhonoured, an undrawn window is plain theme-coloured cells, not an image.
        assert!(id.is_some() || !honor, "{tag}: {name} has no image on screen at ({col},{row}) — the upload never reached the terminal");
        assert_eq!(dominant(&screen, u32::from(col), u32::from(row)), reference, "{tag}: {name} reads like window 2");
    }

    let (mut inside, mut outside, mut kept) = (0, 0, 0);
    let mut bad = Vec::new();
    for row in 0..u32::from(ROWS) {
        for col in 48..u32::from(COLS) {
            let Some(frac) = opaque_fraction(&mask, col, row) else { continue };
            let cell = res.cell(row as u16, col as u16);
            if frac >= 0.9 {
                // Mask-opaque (black) = the bubble. Graphics-window cells (those holding an image) must read like window 2; text cells are window 2's own business.
                inside += usize::from(cell.image_id.is_some());
                if cell.image_id.is_some() && dominant(&screen, col, row) != reference {
                    bad.push(format!("bubble cell ({col},{row}) is {:?}", dominant(&screen, col, row)));
                }
            } else if frac <= 0.1 {
                if cell.image_id.is_some() {
                    // A graphics window's own alpha clip carries the shape (SQ-1711):
                    // its placement is NOT blanked, and the clipped pixels show the
                    // pane, never the bubble's white.
                    kept += 1;
                    if honor && dominant(&screen, col, row) == reference {
                        bad.push(format!("clipped graphics cell ({col},{row}) still reads as the bubble"));
                    }
                } else {
                    // Text windows and filler outside the mask are hidden.
                    outside += 1;
                    if honor && dominant(&screen, col, row) == reference {
                        bad.push(format!("outside cell ({col},{row}) colour={:?}", dominant(&screen, col, row)));
                    }
                }
            }
        }
    }
    assert!(kept > 20 || !honor, "{tag}: non-vacuity, graphics cells past the mask edge keep their placement ({kept})");
    assert!(inside > 150 || !honor, "{tag}: non-vacuity, inside={inside} outside={outside}");
    assert!(bad.is_empty(), "{tag}: {} wrong cells, first: {:?}", bad.len(), &bad[..bad.len().min(40)]);
}

#[cfg(unix)]
#[test]
fn narcolepsy_bubble_is_uniform_on_the_kitty_screen_before_waking() {
    bubble(true, false);
    bubble(false, false);
}

#[cfg(unix)]
#[test]
fn narcolepsy_bubble_is_uniform_on_the_kitty_screen_after_waking() {
    bubble(true, true);
    bubble(false, true);
}
