//! SQ-1707 step A: the design-size fit modes (stretch | aspect) — the one
//! `GlkFit` value in pixels and in cells, the aspect-fit letterbox in the TUI,
//! config precedence, `/set-glk-fit`, the border icon, the mask's per-pixel
//! accessors and the primary text window id.
//!
//! The real-story cases (Photopia 2.01 `photo201.blb`, 640x480; Narcolepsy
//! `narco.blorb`, 800x600) boot through `boot_story` the way startup boots, at
//! the first line prompt (0 inputs submitted), 8x16 cells, and skip vacuously
//! without the fixtures.

use std::path::PathBuf;

use app::config::Config;
use app::engine::{BufferWindow, Engine, WinKind, WinNode};
use app::glk_cfg::{FitRect, GlkFit, GlkFitMode, GlkMask};
use app::host::screen::{
    apply_glk_design, glk_layout, glk_primary_text_window, resize_glulx, run_set_glk_fit,
};
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::render::controls::{controls_for, BorderControl};
use app::session::InputKind;
use app::slash::{self, GlkFitArg, SlashOutcome};
use gvm::glk::WinType;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;

use crate::fixture_paths::fixture_path;

const CELL: (f64, f64) = (8.0, 16.0);

// ── GlkFit, no story needed ─────────────────────────────────────────────────

fn px(mode: GlkFitMode, design: (u32, u32), pane: (u32, u32)) -> GlkFit {
    GlkFit::pixels(mode, design, pane)
}

fn cells(mode: GlkFitMode, design: (u32, u32), pane: (u32, u32)) -> GlkFit {
    GlkFit::cells(mode, design, pane, CELL)
}

fn r(x: u32, y: u32, w: u32, h: u32) -> FitRect {
    FitRect { x, y, w, h }
}

#[test]
fn stretch_frame_is_the_pane_in_both_units() {
    assert_eq!(px(GlkFitMode::Stretch, (640, 480), (1280, 800)).frame(), r(0, 0, 1280, 800));
    assert_eq!(cells(GlkFitMode::Stretch, (640, 480), (100, 37)).frame(), r(0, 0, 100, 37));
    assert!(px(GlkFitMode::Stretch, (640, 480), (1280, 800)).letterbox().is_empty());
}

#[test]
fn aspect_pixels_wide_tall_and_exact() {
    // Wide pane: height-limited, centred horizontally.
    assert_eq!(px(GlkFitMode::Aspect, (640, 480), (1280, 800)).frame(), r(106, 0, 1067, 800));
    // Tall pane: width-limited, centred vertically.
    assert_eq!(px(GlkFitMode::Aspect, (640, 480), (800, 1000)).frame(), r(0, 200, 800, 600));
    // Exact aspect: the frame is the pane.
    assert_eq!(px(GlkFitMode::Aspect, (640, 480), (1280, 960)).frame(), r(0, 0, 1280, 960));
    // Identical scale on both axes (to a pixel of rounding).
    let f = px(GlkFitMode::Aspect, (800, 600), (1280, 800)).frame();
    assert!((f.w as i64 * 600 - f.h as i64 * 800).abs() <= 800);
}

#[test]
fn aspect_cells_wide_tall_and_exact() {
    // 160x30 of 8x16 = 1280x480 device px; 640x480 design: scale 1.0 → 80x30
    // cells, centred in 160 columns.
    assert_eq!(cells(GlkFitMode::Aspect, (640, 480), (160, 30)).frame(), r(40, 0, 80, 30));
    // 70x50 = 560x800: scale 0.875 → 560x420 px = 70 cols x 26.25 rows → 26.
    assert_eq!(cells(GlkFitMode::Aspect, (640, 480), (70, 50)).frame(), r(0, 12, 70, 26));
    // 80x30 = 640x480 exactly the design: the frame is the pane.
    assert_eq!(cells(GlkFitMode::Aspect, (640, 480), (80, 30)).frame(), r(0, 0, 80, 30));
    // Narcolepsy 800x600.
    assert_eq!(cells(GlkFitMode::Aspect, (800, 600), (160, 30)).frame(), r(40, 0, 80, 30));
    assert_eq!(cells(GlkFitMode::Aspect, (800, 600), (70, 50)).frame(), r(0, 12, 70, 26));
}

#[test]
fn frame_and_letterbox_tile_the_pane_exactly() {
    for design in [(640, 480), (800, 600), (1000, 300), (300, 1000)] {
        for pane in [(160, 30), (70, 50), (80, 30), (1, 1), (7, 3), (200, 20), (33, 91), (100, 37)] {
            for fit in [
                cells(GlkFitMode::Aspect, design, pane),
                px(GlkFitMode::Aspect, design, pane),
                cells(GlkFitMode::Stretch, design, pane),
            ] {
                let f = fit.frame();
                assert!(f.w >= 1 && f.h >= 1 && f.x + f.w <= pane.0 && f.y + f.h <= pane.1, "{fit:?} {f:?}");
                let mut cover = vec![0u8; (pane.0 * pane.1) as usize];
                for rc in fit.letterbox().into_iter().chain([f]) {
                    for y in rc.y..rc.y + rc.h {
                        for x in rc.x..rc.x + rc.w {
                            cover[(y * pane.0 + x) as usize] += 1;
                        }
                    }
                }
                assert!(cover.iter().all(|&n| n == 1), "gap or overlap: {fit:?} {f:?}");
                // Centred to within the odd leftover unit.
                let (l, rt) = (f.x, pane.0 - f.x - f.w);
                let (t, b) = (f.y, pane.1 - f.y - f.h);
                assert!(rt - l <= 1 && b - t <= 1, "centred: {fit:?} {f:?}");
            }
        }
    }
}

#[test]
fn window_rects_map_through_the_frame_and_tile_it() {
    let fit = cells(GlkFitMode::Aspect, (640, 480), (160, 30));
    let f = fit.frame();
    // Photopia's four frames abut inside the frame.
    let top = fit.window_rect((0, 0, 640, 58));
    let left = fit.window_rect((0, 58, 14, 343));
    let right = fit.window_rect((626, 58, 14, 343));
    let bottom = fit.window_rect((0, 401, 640, 79));
    assert_eq!((top.x, top.y, top.w), (f.x, f.y, f.w));
    assert_eq!(left.y, top.y + top.h);
    assert_eq!(right.x + right.w, f.x + f.w);
    assert_eq!(bottom.y + bottom.h, f.y + f.h);
    assert_eq!(left.y + left.h, bottom.y);
    // The whole design rect is the frame.
    assert_eq!(fit.window_rect((0, 0, 640, 480)), f);
    // Pixels: same rule.
    let p = px(GlkFitMode::Aspect, (640, 480), (1280, 800));
    assert_eq!(p.window_rect((0, 0, 640, 480)), p.frame());
}

#[test]
fn clicks_map_back_to_design_pixels_and_the_letterbox_goes_nowhere() {
    let fit = cells(GlkFitMode::Aspect, (640, 480), (160, 30)); // frame (40,0,80,30)
    assert_eq!(fit.to_design(40.0, 0.0), Some((0, 0)));
    assert_eq!(fit.to_design(79.99, 29.99), Some((319, 479)));
    assert_eq!(fit.to_design(119.99, 29.99), Some((639, 479)));
    assert_eq!(fit.to_design(80.0, 15.0), Some((320, 240)));
    // The letterbox: left, right, and (in a tall pane) top and bottom.
    assert_eq!(fit.to_design(39.9, 10.0), None);
    assert_eq!(fit.to_design(120.0, 10.0), None);
    let tall = cells(GlkFitMode::Aspect, (640, 480), (70, 50)); // frame (0,12,70,26)
    assert_eq!(tall.to_design(10.0, 11.9), None);
    assert_eq!(tall.to_design(10.0, 38.0), None);
    assert!(tall.to_design(10.0, 12.0).is_some());
    // Stretch has no letterbox: every point of the pane maps.
    let s = cells(GlkFitMode::Stretch, (640, 480), (100, 37));
    assert_eq!(s.to_design(0.0, 0.0), Some((0, 0)));
    assert_eq!(s.to_design(99.99, 36.99), Some((639, 479)));
    assert_eq!(s.to_design(100.0, 0.0), None);
    // Pixels, aspect.
    let p = px(GlkFitMode::Aspect, (640, 480), (800, 1000)); // frame (0,200,800,600)
    assert_eq!(p.to_design(0.0, 199.0), None);
    assert_eq!(p.to_design(400.0, 500.0), Some((320, 240)));
    // The inverse of a window rect's edges: a point just inside lands inside the design rect.
    let w = fit.window_rect((14, 58, 612, 343));
    let (dx, dy) = fit.to_design(w.x as f64 + 0.5, w.y as f64 + 0.5).unwrap();
    assert!((16..=24).contains(&dx) && (64..=80).contains(&dy), "{dx},{dy}");
}

#[test]
fn the_story_is_told_the_text_cell_the_snapped_frame_implies() {
    // 70x50: frame 70x26 cells = 560x416 px for a 640x480 design → the cell is
    // divided by (560/640, 416/480).
    let fit = cells(GlkFitMode::Aspect, (640, 480), (70, 50));
    let s = fit.design_screen(CELL);
    assert_eq!(s, app::glk_cfg::glk_design_screen((640, 480), (560.0, 416.0), CELL));
    // Stretch keeps the whole-pane derivation unchanged.
    let st = cells(GlkFitMode::Stretch, (640, 480), (70, 50));
    assert_eq!(
        st.design_screen(CELL),
        app::glk_cfg::glk_design_screen((640, 480), (560.0, 800.0), CELL)
    );
}

// ── GlkMask per-pixel accessors ─────────────────────────────────────────────

#[test]
fn mask_opaque_and_alpha_image_agree_with_cell_coverage() {
    let (w, h) = (7u32, 5u32);
    let bits: Vec<bool> = (0..w * h).map(|i| (i * 7 + i / 3) % 3 != 0).collect();
    let m = GlkMask::from_opaque(w, h, &bits).unwrap();
    let alpha = m.alpha_image();
    assert_eq!(alpha.dimensions(), (w, h));
    let mut seen = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let want = bits[(y * w + x) as usize];
            assert_eq!(m.opaque(x, y), want, "opaque({x},{y})");
            assert_eq!(alpha.get_pixel(x, y).0[0], if want { 255 } else { 0 });
            // A cols x rows grid the size of the mask has one pixel per cell.
            assert_eq!(m.cell_coverage(x, y, w, h), (want as u32, 1));
            if want { seen.0 += 1 } else { seen.1 += 1 }
        }
    }
    assert!(seen.0 > 0 && seen.1 > 0, "non-vacuity: both opaque and transparent pixels");
    assert!(!m.opaque(w, 0) && !m.opaque(0, h), "outside the picture is not opaque");
}

#[test]
fn narcolepsy_mask_alpha_matches_its_picture() {
    let Some(path) = story("narco.blorb") else { return };
    let b = boot(path, GlkFitMode::Stretch);
    let m = b.state.glk_mask.as_ref().expect("WindowMask loads");
    let a = m.alpha_image();
    assert_eq!(a.dimensions(), (800, 600));
    let opaque = a.pixels().filter(|p| p.0[0] == 255).count();
    assert_eq!(opaque, 401_219, "the black pixels of Pict 3 (see sq1703_glk_mask)");
    assert!(m.opaque(0, 0) && !m.opaque(799, 0));
}

// ── Boot helpers ────────────────────────────────────────────────────────────

fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} missing at {}", p.display());
        return None;
    }
    Some(p)
}

fn boot(path: PathBuf, fit: GlkFitMode) -> BootedStory {
    let home = app::scratch_dir("sq1707-fit");
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg: Config {
            user_dir: home.clone(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            auto_save: false,
            glk_design_fit: fit,
            ..Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("boots headlessly")
}

fn render(b: &mut BootedStory, cols: u16, rows: u16) -> (Buffer, Vec<(u32, WinKind, Rect)>) {
    assert!(resize_glulx(b.session.as_mut(), (cols, rows)));
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = b.session.screen();
    let wins = app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf).win_rects;
    (buf, wins)
}

fn of_kind(r: &[(u32, WinKind, Rect)], k: WinKind) -> Vec<Rect> {
    let mut v: Vec<Rect> = r.iter().filter(|(_, kind, _)| *kind == k).map(|(_, _, r)| *r).collect();
    v.sort_by_key(|r| (r.y, r.x));
    v
}

fn to_rect(f: FitRect) -> Rect {
    Rect::new(f.x as u16, f.y as u16, f.w as u16, f.h as u16)
}

fn assert_first_prompt(b: &BootedStory, design: (u32, u32), fit: GlkFitMode) {
    assert!(b.state.glk_stretch, "the .cfg design size switches design mode on");
    assert_eq!(b.state.glk_fit, fit);
    assert_eq!(b.state.glk_design.as_ref().and_then(|d| d.size()), Some(design));
    assert_eq!(Engine::pending_input(b.session.as_ref()), InputKind::Line, "0 inputs, first line prompt");
}

// ── Real stories, aspect mode ───────────────────────────────────────────────

/// Photopia's frame windows and story text in design pixels.
const P_FRAMES: [(u32, u32, u32, u32); 4] = [(0, 0, 640, 58), (0, 58, 14, 343), (626, 58, 14, 343), (0, 401, 640, 79)];
const P_TEXT: (u32, u32, u32, u32) = (14, 58, 612, 343);

fn aspect_frame(cols: u16, rows: u16, design: (u32, u32)) -> GlkFit {
    GlkFit::cells(GlkFitMode::Aspect, design, (cols as u32, rows as u32), CELL)
}

/// Every cell outside `frame` is a blank in the `glk_mask_outside` style.
fn assert_letterbox_painted(b: &BootedStory, buf: &Buffer, fit: &GlkFit) {
    let outside = b.state.colors.theme.get("glk_mask_outside").style;
    let (of, ob) = (outside.fg.unwrap_or_default(), outside.bg.unwrap_or_default());
    let lb = fit.letterbox();
    assert!(!lb.is_empty(), "non-vacuity: this pane really is letterboxed");
    for rc in lb {
        for y in rc.y..rc.y + rc.h {
            for x in rc.x..rc.x + rc.w {
                let c = &buf[(x as u16, y as u16)];
                assert_eq!((c.symbol(), c.fg, c.bg), (" ", of, ob), "letterbox cell ({x},{y})");
            }
        }
    }
}

fn photopia_aspect(cols: u16, rows: u16) {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path, GlkFitMode::Aspect);
    assert_first_prompt(&b, (640, 480), GlkFitMode::Aspect);
    let (buf, wins) = render(&mut b, cols, rows);
    let fit = aspect_frame(cols, rows, (640, 480));
    let frame = fit.frame();

    let gfx = of_kind(&wins, WinKind::Graphics);
    let text = of_kind(&wins, WinKind::Buffer);
    assert_eq!((gfx.len(), text.len()), (4, 1), "four frame windows and one text window: {wins:?}");
    for d in P_FRAMES {
        let want = to_rect(fit.window_rect(d));
        assert!(gfx.contains(&want), "frame {want:?} missing from {gfx:?} at {cols}x{rows}");
    }
    // The four frames and the text window sit inside the centred frame.
    let fr = to_rect(frame);
    for g in gfx.iter().chain(text.iter()) {
        assert!(fr.contains((g.x, g.y).into()) && g.right() <= fr.right() && g.bottom() <= fr.bottom(), "{g:?} outside {fr:?}");
    }
    // Chars told == cells drawn.
    let l = glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(l.screen, fit.design_screen(CELL), "the story is told the snapped frame's text cell");
    let tw = l.windows.iter().find(|w| w.wintype == WinType::TextBuffer).unwrap();
    assert_eq!((tw.rect.left, tw.rect.top, tw.rect.width, tw.rect.height), P_TEXT);
    let told = (l.screen.chars_in(tw.rect.width, false) as u16, l.screen.chars_in(tw.rect.height, true) as u16);
    assert_eq!((text[0].width, text[0].height), told, "chars told == cells drawn at {cols}x{rows}");
    let slot = to_rect(fit.window_rect(P_TEXT));
    assert_eq!((text[0].x, text[0].y), (slot.x, slot.y));
    assert!(slot.width - text[0].width <= 1 && slot.height - text[0].height <= 1);

    assert_letterbox_painted(&b, &buf, &fit);
    // The frame is centred.
    assert!(frame.x == (cols as u32 - frame.w) / 2 && frame.y == (rows as u32 - frame.h) / 2);
}

#[test]
fn photopia_aspect_in_a_wide_pane() {
    photopia_aspect(160, 30);
}

#[test]
fn photopia_aspect_in_a_tall_pane() {
    photopia_aspect(70, 50);
}

fn narcolepsy_aspect(cols: u16, rows: u16) {
    let Some(path) = story("narco.blorb") else { return };
    let mut b = boot(path, GlkFitMode::Aspect);
    assert_first_prompt(&b, (800, 600), GlkFitMode::Aspect);
    let (buf, wins) = render(&mut b, cols, rows);
    let fit = aspect_frame(cols, rows, (800, 600));
    let frame = fit.frame();
    // Non-vacuity: two text windows and five graphics windows, as in stretch.
    assert_eq!(of_kind(&wins, WinKind::Buffer).len(), 2, "{wins:?}");
    assert_eq!(of_kind(&wins, WinKind::Graphics).len(), 5, "{wins:?}");
    let l = glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(l.screen, fit.design_screen(CELL));
    let story_px = (480, 60, 240, 378);
    let w = l
        .windows
        .iter()
        .find(|w| w.wintype == WinType::TextBuffer && (w.rect.left, w.rect.top, w.rect.width, w.rect.height) == story_px)
        .expect("story text laid out at (480,60,240,378)");
    let slot = to_rect(fit.window_rect(story_px));
    let told = (l.screen.chars_in(w.rect.width, false) as u16, l.screen.chars_in(w.rect.height, true) as u16);
    let want = Rect::new(slot.x, slot.y, told.0.min(slot.width), told.1.min(slot.height));
    assert!(of_kind(&wins, WinKind::Buffer).contains(&want), "story text at {want:?}: {wins:?}");

    assert_letterbox_painted(&b, &buf, &fit);
    // The mask applies inside the frame, over the frame's own extent.
    let mask = b.state.glk_mask.clone().expect("WindowMask loads");
    let outside = b.state.colors.theme.get("glk_mask_outside").style;
    let (of, ob) = (outside.fg.unwrap_or_default(), outside.bg.unwrap_or_default());
    let mut hidden = 0;
    for y in 0..frame.h {
        for x in 0..frame.w {
            if !mask.cell_visible(x, y, frame.w, frame.h) {
                hidden += 1;
                let c = &buf[((frame.x + x) as u16, (frame.y + y) as u16)];
                assert_eq!((c.symbol(), c.fg, c.bg), (" ", of, ob), "masked frame cell ({x},{y})");
            }
        }
    }
    assert!(hidden > 0, "non-vacuity: the bubble hides cells inside the frame");
}

#[test]
fn narcolepsy_aspect_in_a_wide_pane() {
    narcolepsy_aspect(160, 30);
}

#[test]
fn narcolepsy_aspect_in_a_tall_pane() {
    narcolepsy_aspect(70, 50);
}

// ── Image level: each graphics image has its box's aspect in aspect mode ────

fn kitty_px(sym: &str) -> Option<(u32, u32)> {
    let num = |s: &str| -> Option<u32> { s.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok() };
    let g = &sym[sym.find("\x1b_G")?..];
    let ctl = &g[..g.find(';').unwrap_or(g.len())];
    let field = |k: &str| ctl.split(',').find_map(|f| f.trim_start_matches("\x1b_G").strip_prefix(k)).and_then(num);
    Some((field("s=")?, field("v=")?))
}

fn assert_images_match_boxes(name: &str, cols: u16, rows: u16, min_images: usize) {
    let Some(path) = story(name) else { return };
    let mut b = boot(path, GlkFitMode::Aspect);
    assert!(b.state.glk_stretch && b.state.glk_fit == GlkFitMode::Aspect);
    let mut picker = app::render::graphics::kitty_picker(CELL.0 as u16, CELL.1 as u16);
    picker.set_protocol_type(ProtocolType::Kitty);
    b.state.game_picker = Some(picker);
    let (buf, wins) = render(&mut b, cols, rows);
    let mut seen = 0;
    for (_, _, rc) in wins.iter().filter(|(_, k, _)| *k == WinKind::Graphics) {
        let Some(sym) = buf.cell((rc.x, rc.y)).map(|c| c.symbol().to_string()) else { continue };
        let Some((w, h)) = kitty_px(&sym) else { continue };
        seen += 1;
        let (bw, bh) = (i64::from(rc.width) * CELL.0 as i64, i64::from(rc.height) * CELL.1 as i64);
        let off = (i64::from(w) * bh - i64::from(h) * bw).abs();
        assert!(
            off <= bw.max(bh) * 2,
            "{name} {cols}x{rows}: window {rc:?} (box {bw}x{bh}) was handed a {w}x{h} image — aspect-fit would letterbox it"
        );
    }
    assert!(seen >= min_images, "{name} {cols}x{rows}: only {seen} images seen");
}

#[test]
fn photopia_aspect_images_match_their_boxes() {
    for (c, r) in [(160, 30), (70, 50)] {
        assert_images_match_boxes("photo201.blb", c, r, 4);
    }
}

#[test]
fn narcolepsy_aspect_images_match_their_boxes() {
    for (c, r) in [(160, 30), (70, 50)] {
        assert_images_match_boxes("narco.blorb", c, r, 1);
    }
}

// ── Config precedence ───────────────────────────────────────────────────────

#[test]
fn global_config_keys_default_to_design_on_stretch_and_parse() {
    let d = Config::default();
    assert!(d.glk_design);
    assert_eq!(d.glk_design_fit, GlkFitMode::Stretch);
    let c: Config = toml::from_str("glk_design = false\nglk_design_fit = \"aspect\"").unwrap();
    assert!(!c.glk_design);
    assert_eq!(c.glk_design_fit, GlkFitMode::Aspect);
    let typo: Config = toml::from_str("glk_design_fit = \"fill\"").unwrap();
    assert_eq!(typo.glk_design_fit, GlkFitMode::Stretch, "an unknown token reads as the default");
}

#[test]
fn per_game_wins_over_global_and_the_default_is_stretch() {
    use app::glk_cfg::{resolve_design_on, resolve_fit_mode};
    assert_eq!(resolve_fit_mode(None, GlkFitMode::Stretch), GlkFitMode::Stretch);
    assert_eq!(resolve_fit_mode(None, GlkFitMode::Aspect), GlkFitMode::Aspect);
    assert_eq!(resolve_fit_mode(Some(GlkFitMode::Stretch), GlkFitMode::Aspect), GlkFitMode::Stretch);
    assert_eq!(resolve_fit_mode(Some(GlkFitMode::Aspect), GlkFitMode::Stretch), GlkFitMode::Aspect);
    assert!(resolve_design_on(None, true));
    assert!(!resolve_design_on(None, false));
    assert!(resolve_design_on(Some(true), false));
    assert!(!resolve_design_on(Some(false), true));
}

#[test]
fn boot_resolves_the_fit_and_the_design_switch_per_game_over_global() {
    let Some(path) = story("photo201.blb") else { return };
    // Default global: stretch.
    let mut b = boot(path.clone(), GlkFitMode::Stretch);
    assert_first_prompt(&b, (640, 480), GlkFitMode::Stretch);
    let dir = b.game_dir.clone();
    // Per-game aspect over a stretch global.
    app::styles::write_per_game_glk_design_fit(&dir, Some(GlkFitMode::Aspect)).unwrap();
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert_eq!(b.state.glk_fit, GlkFitMode::Aspect);
    assert_eq!(app::host::screen::glk_fit(b.session.as_mut()).unwrap().mode, GlkFitMode::Aspect);
    // Cleared: the global decides again.
    app::styles::write_per_game_glk_design_fit(&dir, None).unwrap();
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert_eq!(b.state.glk_fit, GlkFitMode::Stretch);
    // Global aspect, per-game stretch wins.
    b.state.config.glk_design_fit = GlkFitMode::Aspect;
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert_eq!(b.state.glk_fit, GlkFitMode::Aspect, "global applies when no per-game key");
    app::styles::write_per_game_glk_design_fit(&dir, Some(GlkFitMode::Stretch)).unwrap();
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert_eq!(b.state.glk_fit, GlkFitMode::Stretch, "per-game wins");
    app::styles::write_per_game_glk_design_fit(&dir, None).unwrap();
    // The design switch: global off turns design layout off; per-game on beats it.
    b.state.config.glk_design = false;
    assert!(!apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert!(!b.state.glk_stretch);
    assert_eq!(app::host::screen::glk_design_size(&b.state, &dir), None);
    let mut pg = app::styles::PerGameConfig::read(&dir);
    pg.glk_design = Some(true);
    pg.write(&dir).unwrap();
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert_eq!(app::host::screen::glk_design_size(&b.state, &dir), Some((640, 480)));
    let _ = std::fs::remove_file(app::styles::per_game_config_path(&dir));
}

fn design_off_returns_the_running_session_to_cell_layout(name: &str, design: (u32, u32), masked: bool) {
    let Some(path) = story(name) else { return };
    let mut b = boot(path, GlkFitMode::Stretch);
    assert_first_prompt(&b, design, GlkFitMode::Stretch);
    let dir = b.game_dir.clone();
    let _ = render(&mut b, 100, 40);
    // Non-vacuity: design mode is really on in the session.
    assert!(app::host::screen::glk_fit(b.session.as_mut()).is_some());
    assert_eq!(glk_layout(b.session.as_mut()).unwrap().screen.size, design);
    let had_mask = b.state.glk_mask.is_some();
    assert!(had_mask || !masked, "non-vacuity: this story's mask loads");

    b.state.config.glk_design = false;
    assert!(!apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert!(!b.state.glk_stretch);
    assert!(b.state.glk_mask.is_none());
    assert!(app::host::screen::glk_fit(b.session.as_mut()).is_none(), "session left design mode");
    let _ = render(&mut b, 100, 40);
    let l = glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(l.screen.size, (100, 40), "cell tree: layout units are the pane's cells");
    assert_eq!(l.screen.text_cell, (1.0, 1.0));

    b.state.config.glk_design = true;
    assert!(apply_glk_design(b.session.as_mut(), &mut b.state, &dir));
    assert!(b.state.glk_stretch);
    assert_eq!(b.state.glk_mask.is_some(), had_mask, "the mask returns with design mode");
    let _ = render(&mut b, 100, 40);
    assert_eq!(glk_layout(b.session.as_mut()).unwrap().screen.size, design, "design mode returns");
}

#[test]
fn photopia_design_off_returns_to_cell_layout() {
    design_off_returns_the_running_session_to_cell_layout("photo201.blb", (640, 480), false);
}

#[test]
fn narcolepsy_design_off_returns_to_cell_layout_and_clears_the_mask() {
    design_off_returns_the_running_session_to_cell_layout("narco.blorb", (800, 600), true);
}

// ── /set-glk-fit ────────────────────────────────────────────────────────────

#[test]
fn set_glk_fit_parses() {
    let p = |s: &str| slash::parse(s, '/');
    assert!(matches!(p("set-glk-fit"), SlashOutcome::SetGlkFit(GlkFitArg::Toggle)));
    assert!(matches!(p("set-glk-fit stretch"), SlashOutcome::SetGlkFit(GlkFitArg::Mode(GlkFitMode::Stretch))));
    assert!(matches!(p("set-glk-fit aspect"), SlashOutcome::SetGlkFit(GlkFitArg::Mode(GlkFitMode::Aspect))));
    assert!(matches!(p("set-glk-fit auto"), SlashOutcome::SetGlkFit(GlkFitArg::Auto)));
    assert!(matches!(p("set-glk-fit squash"), SlashOutcome::Error(_)));
    assert!(slash::COMMANDS.iter().any(|c| c.name == "set-glk-fit"), "registered in COMMANDS");
}

#[test]
fn set_glk_fit_toggles_sets_persists_and_autos() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path, GlkFitMode::Stretch);
    assert_first_prompt(&b, (640, 480), GlkFitMode::Stretch);
    let dir = b.game_dir.clone();
    let _ = std::fs::remove_file(app::styles::per_game_config_path(&dir));
    let s = b.session.as_mut();

    // Bare toggles stretch → aspect, persisted per-game and applied live.
    let line = run_set_glk_fit(s, &mut b.state, &dir, GlkFitArg::Toggle).unwrap();
    assert!(line.contains("aspect"), "{line}");
    assert_eq!(b.state.glk_fit, GlkFitMode::Aspect);
    assert_eq!(app::styles::read_per_game_glk_design_fit(&dir), Some(GlkFitMode::Aspect));
    assert_eq!(app::host::screen::glk_fit(b.session.as_mut()).unwrap().mode, GlkFitMode::Aspect);
    // …and the frame on screen really is letterboxed now.
    let (buf, _) = render(&mut b, 160, 30);
    assert_letterbox_painted(&b, &buf, &aspect_frame(160, 30, (640, 480)));

    // Again: back to stretch.
    run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Toggle).unwrap();
    assert_eq!(b.state.glk_fit, GlkFitMode::Stretch);
    assert_eq!(app::styles::read_per_game_glk_design_fit(&dir), Some(GlkFitMode::Stretch));

    // Explicit.
    run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Mode(GlkFitMode::Aspect)).unwrap();
    assert_eq!(b.state.glk_fit, GlkFitMode::Aspect);

    // Auto clears the key and inherits the global value.
    b.state.config.glk_design_fit = GlkFitMode::Stretch;
    let line = run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Auto).unwrap();
    assert!(line.contains("auto"), "{line}");
    assert_eq!(app::styles::read_per_game_glk_design_fit(&dir), None, "the key is gone");
    assert_eq!(b.state.glk_fit, GlkFitMode::Stretch);
    let _ = std::fs::remove_file(app::styles::per_game_config_path(&dir));
}

#[test]
fn set_glk_fit_on_a_game_without_a_design_size_says_so_and_changes_nothing() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path, GlkFitMode::Stretch);
    let dir = b.game_dir.clone();
    let _ = std::fs::remove_file(app::styles::per_game_config_path(&dir));
    b.state.glk_design = None; // as for a story with no .cfg
    let e = run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Toggle).unwrap_err();
    assert_eq!(e, "this game has no design size");
    assert_eq!(b.state.glk_fit, GlkFitMode::Stretch);
    assert_eq!(app::styles::read_per_game_glk_design_fit(&dir), None, "nothing persisted");
    // A .cfg without both dimensions is no design size either.
    b.state.glk_design = Some(app::glk_cfg::GlkDesign::parse("WindowWidth=800\n"));
    assert!(run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Toggle).is_err());
}

// ── The border icon ─────────────────────────────────────────────────────────

fn render_control(state: &app::state::AppState) -> Option<app::render::controls::ControlView> {
    controls_for(state).into_iter().find(|v| v.id == BorderControl::V6Render)
}

#[test]
fn the_icon_shows_only_with_a_design_size_and_runs_set_glk_fit() {
    let mut st = app::state::AppState::default();
    assert!(render_control(&st).is_none(), "no design size, not v6: no icon");
    assert_eq!(BorderControl::V6Render.command_for(&st).name, "set-v6-render");

    st.glk_stretch = true;
    st.glk_fit = GlkFitMode::Stretch;
    let stretch = render_control(&st).expect("icon with a design size");
    assert!(stretch.hint.iter().any(|h| h == "/set-glk-fit"), "{:?}", stretch.hint);
    assert!(stretch.hint[0].contains("stretch") && stretch.hint[0].contains("aspect"), "{:?}", stretch.hint);
    assert_eq!(BorderControl::V6Render.command_for(&st).to_string(), "set-glk-fit");

    st.glk_fit = GlkFitMode::Aspect;
    let aspect = render_control(&st).unwrap();
    assert_ne!(stretch.glyph, aspect.glyph, "the glyph tells the two modes apart");
    assert!(aspect.hint[0].contains("aspect"), "{:?}", aspect.hint);

    // v6 behaviour unchanged: with no design size a v6 story gets its render control.
    let mut v6 = app::state::AppState::default();
    v6.story_zversion = Some(6);
    let c = render_control(&v6).expect("v6 render control");
    assert!(c.hint.iter().any(|h| h == "/set-v6-render"), "{:?}", c.hint);
    assert_eq!(BorderControl::V6Render.command_for(&v6).name, "set-v6-render");
}

#[test]
fn the_icon_follows_a_booted_story() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = boot(path, GlkFitMode::Aspect);
    let v = render_control(&b.state).expect("icon on a design-size story");
    assert!(v.hint[0].contains("aspect"));
    let dir = b.game_dir.clone();
    run_set_glk_fit(b.session.as_mut(), &mut b.state, &dir, GlkFitArg::Toggle).unwrap();
    assert!(render_control(&b.state).unwrap().hint[0].contains("Fit: stretch"));
    let _ = std::fs::remove_file(app::styles::per_game_config_path(&dir));
    // Design layout off: the icon is gone.
    b.state.config.glk_design = false;
    apply_glk_design(b.session.as_mut(), &mut b.state, &dir);
    assert!(render_control(&b.state).is_none());
}

// ── Primary text window ─────────────────────────────────────────────────────

fn primary_in(node: &WinNode) -> Option<u32> {
    match node {
        WinNode::Buffer(BufferWindow { primary: true, win, .. }) => Some(*win),
        WinNode::Pair { first, second, .. } => primary_in(first).or_else(|| primary_in(second)),
        _ => None,
    }
}

#[test]
fn primary_text_window_is_the_story_window() {
    for (name, story_px) in [("photo201.blb", (14, 58, 612, 343)), ("narco.blorb", (480, 60, 240, 378))] {
        let Some(path) = story(name) else { continue };
        let mut b = boot(path, GlkFitMode::Stretch);
        let id = glk_primary_text_window(b.session.as_mut()).expect("a primary text window");
        let l = glk_layout(b.session.as_mut()).unwrap();
        let w = l.windows.iter().find(|w| w.id == id).expect("it is a laid-out window");
        assert_eq!(w.wintype, WinType::TextBuffer);
        assert_eq!((w.rect.left, w.rect.top, w.rect.width, w.rect.height), story_px, "{name}: the story text window");
        // The same window the renderer treats as primary.
        assert_eq!(primary_in(&b.session.screen().root), Some(id), "{name}");
    }
}
