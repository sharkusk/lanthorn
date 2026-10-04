//! SQ-1703 P4: the Windows Glk window mask (`WindowMask=<pict>`) in stretch
//! mode. Windows Glulxe's `config.htm`: "If a particular pixel in the graphic
//! is white then the window is transparent at that point, else it is opaque."
//!
//! Real specimens (skip vacuously without `stories/`), booted through
//! `boot_story`, 0 inputs, at the first line prompt: Narcolepsy
//! (`narco.blorb`, Pict 3, 800x600 — left half opaque, right half a
//! thought-bubble) and Photopia 2.01 (`photo201.blb`, Pict 39, 640x480 —
//! rounded corners only).

use std::path::PathBuf;

use app::config::Config;
use app::engine::Engine;
use app::graphics::PictSource;
use app::host::screen::resize_glulx;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} missing at {}", p.display());
        return None;
    }
    Some(p)
}

fn boot(path: PathBuf) -> BootedStory {
    let home = app::scratch_dir("sq1703-mask");
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
            ..Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("boots headlessly")
}

fn render(b: &mut BootedStory, cols: u16, rows: u16) -> Buffer {
    assert!(resize_glulx(b.session.as_mut(), (cols, rows)));
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = b.session.screen();
    app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf);
    buf
}

fn text_at(buf: &Buffer, x: u16, y: u16, n: u16) -> String {
    (x..x + n).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

/// The mask Pict decoded straight from the Blorb, independent of the
/// loader under test: `(width, height, black, white, other)`.
fn pict_census(path: &std::path::Path, resnum: u32) -> (u32, u32, u32, u32, u32) {
    let img = PictSource::resolve(path, None).image(resnum).expect("mask Pict decodes").to_rgba8();
    let (mut black, mut white, mut other) = (0, 0, 0);
    for p in img.pixels() {
        match p.0 {
            [0, 0, 0, 255] => black += 1,
            [255, 255, 255, 255] => white += 1,
            _ => other += 1,
        }
    }
    (img.width(), img.height(), black, white, other)
}

fn booted_first_prompt(path: PathBuf, stretch_size: (u32, u32)) -> BootedStory {
    let b = boot(path);
    assert!(b.state.glk_stretch, "the .cfg design size switches stretch mode on");
    assert_eq!(b.state.glk_design.as_ref().and_then(|d| d.size()), Some(stretch_size));
    assert_eq!(Engine::pending_input(b.session.as_ref()), InputKind::Line, "0 inputs, first line prompt");
    b
}

#[test]
fn narcolepsy_mask_is_pict_3_white_transparent_black_opaque() {
    let Some(path) = story("narco.blorb") else { return };
    // The facts the semantics rest on, read from the blorb itself.
    assert_eq!(pict_census(&path, 3), (800, 600, 401_219, 78_781, 0), "pure black and white");
    let b = booted_first_prompt(path, (800, 600));
    let m = b.state.glk_mask.as_ref().expect("WindowMask=3 loads");
    assert_eq!(m.size(), (800, 600));
    // 100x37 cells over 800x600: cell (0,0) spans x 0..8, y 0..16 of the left
    // opaque half, so it is wholly black.
    assert_eq!(m.cell_coverage(0, 0, 100, 37), (8 * 16, 8 * 16));
}

#[test]
fn narcolepsy_hides_the_cells_outside_the_bubble_and_draws_the_rest() {
    let Some(path) = story("narco.blorb") else { return };
    let mut b = booted_first_prompt(path, (800, 600));
    let masked = render(&mut b, 100, 37);
    let mask = b.state.glk_mask.take().unwrap();
    let unmasked = render(&mut b, 100, 37);
    let outside = b.state.colors.theme.get("glk_mask_outside").style;

    // Pinned cells: the whole left half is in, above and below the bubble are
    // out, the bubble's interior is in.
    assert!(mask.cell_visible(0, 0, 100, 37) && mask.cell_visible(49, 36, 100, 37), "left half opaque");
    assert!(!mask.cell_visible(99, 0, 100, 37) && !mask.cell_visible(60, 0, 100, 37), "above the bubble");
    assert!(!mask.cell_visible(99, 36, 100, 37) && !mask.cell_visible(75, 35, 100, 37), "below the bubble");
    assert!(mask.cell_visible(75, 15, 100, 37), "bubble interior");
    let hidden = mask.visible_cells(100, 37).iter().filter(|&&v| !v).count();
    assert_eq!(hidden, 611, "non-vacuity: a real bubble, not all-in or all-out");

    for y in 0..37u16 {
        for x in 0..100u16 {
            let (m, u) = (&masked[(x, y)], &unmasked[(x, y)]);
            if mask.cell_visible(x as u32, y as u32, 100, 37) {
                assert_eq!(
                    (m.symbol(), m.fg, m.bg),
                    (u.symbol(), u.fg, u.bg),
                    "visible cell ({x},{y}) is drawn as without a mask"
                );
            } else {
                assert_eq!((m.symbol(), m.fg, m.bg), (" ", outside.fg.unwrap_or_default(), outside.bg.unwrap_or_default()), "hidden cell ({x},{y}) shows the pane");
            }
        }
    }
    // Non-vacuity: with no mask something else was drawn out there, so the
    // comparison above proves the mask did the hiding.
    let (of, ob) = (outside.fg.unwrap_or_default(), outside.bg.unwrap_or_default());
    assert!((0..37u16).any(|y| (0..100u16).any(|x| !mask.cell_visible(x as u32, y as u32, 100, 37) && (unmasked[(x, y)].fg, unmasked[(x, y)].bg) != (of, ob))), "the fill without the mask differs from the outside style");

    // The story text window (480,60 240x378 -> cells 60,4 30x23) is inside the
    // bubble: its text is drawn, on both sides of the mask.
    assert_eq!(text_at(&masked, 60, 5, 5), "Awake");
    assert_eq!(text_at(&masked, 60, 15, 1), ">");
    assert_eq!(text_at(&unmasked, 60, 5, 5), "Awake");
    // Presentation only: the story is told the same screen either way.
    let told = app::host::screen::glk_layout(b.session.as_mut()).unwrap();
    assert_eq!(told.screen.size, (800, 600));
}

#[test]
fn photopia_mask_only_rounds_the_corners() {
    let Some(path) = story("photo201.blb") else { return };
    // 32 white pixels in a 640x480 black field: the four corners, sub-cell.
    let (w, h, _black, white, _other) = pict_census(&path, 39);
    assert_eq!((w, h, white), (640, 480, 32));
    let mut b = booted_first_prompt(path, (640, 480));
    let m = b.state.glk_mask.clone().expect("WindowMask=39 loads");
    assert_eq!(m.size(), (640, 480));
    // At real pane sizes the corners are a sliver of a cell: nothing hidden.
    for (c, r) in [(100u32, 37u32), (160, 45), (200, 20), (80, 24)] {
        assert!(m.visible_cells(c, r).iter().all(|&v| v), "no cell hidden at {c}x{r}");
    }
    // Only where a cell is a couple of mask pixels does a corner cell lose
    // more than half: 320x240 cells are 2x2 px.
    let v = m.visible_cells(320, 240);
    let hidden: Vec<usize> = (0..v.len()).filter(|&i| !v[i]).collect();
    assert!(!hidden.is_empty() && hidden.len() <= 16, "only corner cells: {hidden:?}");
    assert!(!m.cell_visible(0, 0, 320, 240), "the top-left corner cell is mostly outside");
    assert!(m.cell_visible(160, 120, 320, 240));
    // Rendering at a normal size hides nothing: identical to no mask at all.
    let masked = render(&mut b, 100, 37);
    b.state.glk_mask = None;
    let unmasked = render(&mut b, 100, 37);
    assert_eq!(masked, unmasked, "the sub-cell corners change no cell at 100x37");
}

#[test]
fn a_story_canvas_is_clipped_to_the_mask_at_pixel_level() {
    let Some(path) = story("narco.blorb") else { return };
    let b = booted_first_prompt(path, (800, 600));
    let m = b.state.glk_mask.as_ref().unwrap();
    // A canvas covering the whole frame at design resolution loses exactly the
    // transparent pixels.
    let full = image::RgbaImage::from_pixel(800, 600, image::Rgba([200, 10, 10, 255]));
    let clipped = m.clip_canvas(&full, (0, 0, 800, 600), (800, 600)).expect("the bubble clips");
    let clear = clipped.pixels().filter(|p| p.0[3] == 0).count();
    assert_eq!(clear, 78_781, "exactly the white pixels of Pict 3 become transparent");
    // The left (opaque) half is untouched: a window there needs no clipping.
    let left = image::RgbaImage::from_pixel(400, 80, image::Rgba([1, 2, 3, 255]));
    assert!(m.clip_canvas(&left, (0, 0, 400, 80), (800, 600)).is_none());
}

#[test]
fn the_per_game_switch_keeps_cell_mode_without_a_mask() {
    let Some(path) = story("narco.blorb") else { return };
    let home = app::scratch_dir("sq1703-mask-switch");
    let overrides = LaunchOverrides::default();
    let mk = |home: &PathBuf| BootRequest {
        story_path: path.clone(),
        disk_entry: None,
        overrides: &overrides,
        cfg: Config {
            user_dir: home.clone(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            auto_save: false,
            ..Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    let on = boot_story(mk(&home), &mut QuietBoot).unwrap();
    assert!(on.state.glk_mask.is_some());
    let side = app::styles::PerGameConfig { glk_design: Some(false), ..Default::default() };
    side.write(&on.game_dir).unwrap();
    let off = boot_story(mk(&home), &mut QuietBoot).unwrap();
    assert!(!off.state.glk_stretch && off.state.glk_mask.is_none(), "no mask in cell mode");
}

fn canvases(n: &app::engine::WinNode, out: &mut Vec<(u32, std::sync::Arc<image::RgbaImage>)>) {
    match n {
        app::engine::WinNode::Graphics(g) => out.push((g.win, g.canvas.clone())),
        app::engine::WinNode::Pair { first, second, .. } => {
            canvases(first, out);
            canvases(second, out);
        }
        _ => {}
    }
}

#[test]
fn the_session_clips_graphics_window_canvases_it_hands_the_renderer() {
    let Some(path) = story("photo201.blb") else { return };
    let mut b = booted_first_prompt(path, (640, 480));
    assert!(resize_glulx(b.session.as_mut(), (100, 37)));
    let (mut with, mut without) = (Vec::new(), Vec::new());
    canvases(&b.session.screen().root, &mut with);
    let gs = b.session.as_any_mut().downcast_mut::<app::glulx_session::GlulxSession>().unwrap();
    assert!(gs.set_glk_mask(None).is_none());
    canvases(&b.session.screen().root, &mut without);
    assert_eq!(with.len(), 4, "four graphics windows");
    assert_eq!(with.len(), without.len());
    let clear = |c: &image::RgbaImage| c.pixels().filter(|p| p.0[3] == 0).count();
    // Photopia's top (640x58) and bottom (640x79) frames hold the four mask
    // corners, 8 transparent pixels each: 16 per frame once clipped. The side
    // frames (14x343) are wholly inside the mask and are the same either way.
    let by_win = |v: &[(u32, std::sync::Arc<image::RgbaImage>)], w: u32| v.iter().find(|c| c.0 == w).unwrap().1.clone();
    for (win, dims) in [(2u32, (640u32, 58u32)), (4, (640, 79))] {
        let (m, u) = (by_win(&with, win), by_win(&without, win));
        assert_eq!((m.width(), m.height()), dims);
        assert_eq!((clear(&m), clear(&u)), (16, 0), "frame {win} loses exactly its corner pixels");
    }
    for win in [6u32, 8] {
        assert_eq!(by_win(&with, win), by_win(&without, win), "side frame {win} untouched");
    }
}

/// SQ-1711: Narcolepsy's thought bubble is text window 2 plus four graphics
/// windows the game never draws into. Glk's initial graphics background is
/// white, so every mask-visible cell of the right half must match window 2's background (white when game colours are honoured, the theme's when not), both
/// before waking (0 inputs; primary = window 2) and after (2 inputs: `wake up`
/// plus a key; primary = window 1, navy), under both `honor_game_colours`.
#[test]
fn narcolepsy_undrawn_graphics_windows_are_white_before_and_after_waking() {
    let Some(path) = story("narco.blorb") else { return };
    for honor in [true, false] {
        for inputs in [0usize, 2] {
            let home = app::scratch_dir("sq1711-white");
            let overrides = LaunchOverrides::default();
            let req = BootRequest {
                story_path: path.clone(),
                disk_entry: None,
                overrides: &overrides,
                cfg: Config {
                    user_dir: home.clone(),
                    config_file: home.join("config.toml"),
                    random_seed: Some(1),
                    auto_save: false,
                    honor_game_colours: honor,
                    ..Config::default()
                },
                roots: app::data_roots::DataRoots::single(home.join("saves")),
                flags: LaunchFlags::default(),
                terminal: TerminalFacts::default(),
                fresh_start: true,
            };
            let mut b = boot_story(req, &mut QuietBoot).expect("boots headlessly");
            if inputs == 2 {
                let _ = Engine::submit(b.session.as_mut(), "wake up");
                let _ = Engine::submit_key(b.session.as_mut(), app::engine::KeyInput::Char(' '));
            }
            let buf = render(&mut b, 100, 37);
            let mask = b.state.glk_mask.clone().unwrap();
            // Window 2 (480,60 240x378 design px = cells 60,4 30x23): the text
            // window of the bubble. Its bg is the reference in BOTH modes.
            let reference = buf[(89, 25)].bg;
            if honor {
                assert_eq!(reference, ratatui::style::Color::Rgb(255, 255, 255), "honoured: window 2 is Glk white");
            }
            if !honor {
                // The theme's window background, which every text window gets
                // with game colours off (not the game's white).
                assert_eq!(reference, b.state.colors.theme.get("transcript").style.bg.unwrap_or_default(), "unhonoured: window 2 is the theme's");
            }
            let white = reference;
            let (mut seen, mut bad, mut navy) = (0, Vec::new(), 0);
            for y in 0..37u16 {
                for x in 0..100u16 {
                    if !mask.cell_visible(x as u32, y as u32, 100, 37) {
                        continue;
                    }
                    if x >= 50 {
                        seen += 1;
                        if buf[(x, y)].bg != white {
                            bad.push((x, y, buf[(x, y)].bg));
                        }
                    } else if buf[(x, y)].bg == ratatui::style::Color::Rgb(0, 0, 0x80) {
                        navy += 1;
                    }
                }
            }
            assert!(seen > 300, "non-vacuity: the bubble has many visible cells ({seen})");
            assert!(bad.is_empty(), "honor={honor} inputs={inputs}: cells not matching window 2 {:?}", &bad[..bad.len().min(8)]);
            if inputs == 2 && honor {
                assert!(navy > 100, "non-vacuity: window 1 is navy after waking ({navy})");
            }
        }
    }
}
