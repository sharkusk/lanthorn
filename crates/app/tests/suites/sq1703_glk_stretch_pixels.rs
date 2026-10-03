//! SQ-1703 stretch mode, at the IMAGE level: the pixels handed to the terminal
//! for each graphics window have the aspect of that window's cell box.
//!
//! The cell-buffer suites (`sq1703_glk_stretch`) pass on a frame that is wrong on
//! screen: kitty fits an image into a unicode-placeholder `r x c` box with its
//! aspect ratio PRESERVED ("The image will eventually be fit to the specified
//! rectangle, its aspect ratio preserved" —
//! <https://sw.kovidgoyal.net/kitty/graphics-protocol/>), so a raw 640x58 canvas
//! in a 160x2-cell box is drawn short of the window and centred. These cases
//! read the transmitted image dimensions out of the bytes the buffer carries
//! (kitty `s=W,v=H`; sixel raster attributes `"1;1;W;H`).
//!
//! Skip vacuously without `stories/`.

use std::path::PathBuf;

use app::config::Config;
use app::engine::WinKind;
use app::host::screen::resize_glulx;
use app::host::{boot_story, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;

use crate::fixture_paths::fixture_path;

const CELL: (u16, u16) = (8, 16);

fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} missing at {}", p.display());
        return None;
    }
    Some(p)
}

fn boot(path: PathBuf) -> BootedStory {
    let home = app::scratch_dir("sq1703-stretch-px");
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

/// `(rect, image pixel size)` for every graphics window drawn as an image.
fn transmitted(path: PathBuf, proto: ProtocolType, cols: u16, rows: u16, input: &[&str]) -> Vec<(Rect, (u32, u32))> {
    let mut b = boot(path);
    assert!(b.state.glk_stretch, "design mode is on");
    let mut picker = app::render::graphics::kitty_picker(CELL.0, CELL.1);
    picker.set_protocol_type(proto);
    b.state.game_picker = Some(picker);
    assert!(resize_glulx(b.session.as_mut(), (cols, rows)));
    for line in input {
        let _ = app::engine::Engine::submit(b.session.as_mut(), line);
    }
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = b.session.screen();
    let wins = app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf).win_rects;
    let mut out = Vec::new();
    for (_, _, r) in wins.iter().filter(|(_, k, _)| *k == WinKind::Graphics) {
        // The transmit rides the window's first cell.
        let Some(sym) = buf.cell((r.x, r.y)).map(|c| c.symbol().to_string()) else { continue };
        if let Some(px) = image_px(&sym, proto) {
            out.push((*r, px));
        }
    }
    out
}

fn image_px(sym: &str, proto: ProtocolType) -> Option<(u32, u32)> {
    let num = |s: &str| -> Option<u32> { s.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok() };
    match proto {
        ProtocolType::Kitty => {
            let g = &sym[sym.find("\x1b_G")?..];
            let ctl = &g[..g.find(';').unwrap_or(g.len())];
            let field = |k: &str| ctl.split(',').find_map(|f| f.trim_start_matches("\x1b_G").strip_prefix(k)).and_then(num);
            Some((field("s=")?, field("v=")?))
        }
        _ => sixel_px(sym),
    }
}

/// A sixel stream's pixel size, counted from the data (this encoder writes no
/// raster attributes): width is the longest `$`-separated run of sixel
/// characters in a band, height is six rows per `-`-separated band.
fn sixel_px(sym: &str) -> Option<(u32, u32)> {
    let data = &sym[sym.find("\x1bP")?..];
    let data = &data[data.find('q')? + 1..];
    let data = &data[..data.find('\x1b').unwrap_or(data.len())];
    let bands: Vec<&str> = data.split('-').filter(|b| b.chars().any(|c| ('?'..='~').contains(&c))).collect();
    let mut width = 0u32;
    for band in &bands {
        for seg in band.split('$') {
            let (mut n, mut chars) = (0u32, seg.chars().peekable());
            while let Some(c) = chars.next() {
                match c {
                    '#' => {
                        while chars.peek().is_some_and(|d| d.is_ascii_digit() || *d == ';') {
                            chars.next();
                        }
                    }
                    '!' => {
                        let mut k = 0u32;
                        while let Some(d) = chars.peek().and_then(|d| d.to_digit(10)) {
                            k = k * 10 + d;
                            chars.next();
                        }
                        chars.next();
                        n += k;
                    }
                    '?'..='~' => n += 1,
                    _ => {}
                }
            }
            width = width.max(n);
        }
    }
    Some((width, bands.len() as u32 * 6))
}

fn assert_stretched(name: &str, proto: ProtocolType, cols: u16, rows: u16, min_images: usize) {
    assert_stretched_after(name, proto, cols, rows, min_images, &[]);
}

fn assert_stretched_after(name: &str, proto: ProtocolType, cols: u16, rows: u16, min_images: usize, input: &[&str]) {
    let Some(path) = story(name) else { return };
    let imgs = transmitted(path, proto, cols, rows, input);
    assert!(imgs.len() >= min_images, "{name} {proto:?} {cols}x{rows}: only {} images seen: {imgs:?}", imgs.len());
    for (r, (w, h)) in imgs {
        let (bw, bh) = (i64::from(r.width) * i64::from(CELL.0), i64::from(r.height) * i64::from(CELL.1));
        // Sixel pads its last band to six rows.
        let h = if proto == ProtocolType::Sixel && i64::from(h) - bh < 6 { h.min(bh as u32) } else { h };
        // Aspect equal to the box's up to a pixel of rounding per axis: aspect-fit is then a no-op.
        let off = (i64::from(w) * bh - i64::from(h) * bw).abs();
        assert!(
            off <= bw.max(bh) * 2,
            "{name} {proto:?} {cols}x{rows}: window {r:?} (box {bw}x{bh}) was handed a {w}x{h} image — aspect-fit would letterbox it"
        );
    }
}

#[test]
fn photopia_kitty_images_match_their_boxes() {
    for (c, r) in [(160, 30), (70, 50), (100, 37)] {
        assert_stretched("photo201.blb", ProtocolType::Kitty, c, r, 4);
    }
}

/// The chapter card: after `n` Photopia draws its 612x343 black card into a
/// graphics window that covers the whole story-text area. Aspect-fit in a tall
/// pane that card was a 528x296 image centred in a 528x576 box, so it covered
/// only a band of the text area (the likeliest source of the band the owner saw
/// in Ghostty, which no stream-level capture of the idle frame reproduces);
/// stretched, the image fills its box.
#[test]
fn photopia_chapter_card_fills_the_text_area() {
    let Some(path) = story("photo201.blb") else { return };
    for (c, r) in [(70, 50), (160, 30)] {
        let imgs = transmitted(path.clone(), ProtocolType::Kitty, c, r, &["n"]);
        // Non-vacuity: the card is the one window that is not a frame — it sits
        // inside the frames, and is far bigger than a side bar.
        let card = imgs.iter().find(|(rc, _)| rc.x > 0 && rc.width > 20).unwrap_or_else(|| panic!("no card window: {imgs:?}"));
        let (bw, bh) = (u32::from(card.0.width) * u32::from(CELL.0), u32::from(card.0.height) * u32::from(CELL.1));
        assert_eq!(card.1, (bw, bh), "{c}x{r}: the card is handed to kitty at exactly its box");
    }
    assert_stretched_after("photo201.blb", ProtocolType::Kitty, 70, 50, 5, &["n"]);
}

#[test]
fn photopia_sixel_images_match_their_boxes() {
    for (c, r) in [(160, 30), (70, 50)] {
        assert_stretched("photo201.blb", ProtocolType::Sixel, c, r, 4);
    }
}

/// Narcolepsy's left column — the 80 design-px title window over the blue root
/// text window — abuts with no gap at any pane size (the dark gap the user saw
/// below the title was the title art letterboxed inside a correctly placed box).
#[test]
fn narcolepsy_title_and_blue_region_abut() {
    let Some(path) = story("narco.blorb") else { return };
    for (cols, rows) in [(80u16, 50u16), (100, 37), (60, 70), (160, 30)] {
        let mut b = boot(path.clone());
        assert!(resize_glulx(b.session.as_mut(), (cols, rows)));
        let area = Rect::new(0, 0, cols, rows);
        let mut buf = Buffer::empty(area);
        let model = b.session.screen();
        let wins = app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf).win_rects;
        let mut col0: Vec<Rect> = wins.iter().map(|(_, _, r)| *r).filter(|r| r.x == 0 && r.width > 0).collect();
        col0.sort_by_key(|r| r.y);
        assert!(col0.len() >= 2, "title + blue region at {cols}x{rows}: {wins:?}");
        assert_eq!(col0[0].y, 0, "title starts at the top");
        for w in col0.windows(2) {
            assert!(w[1].y <= w[0].bottom(), "gap between {:?} and {:?} at {cols}x{rows}", w[0], w[1]);
        }
    }
}

#[test]
fn narcolepsy_kitty_images_match_their_boxes() {
    for (c, r) in [(80, 50), (100, 37), (160, 30)] {
        assert_stretched("narco.blorb", ProtocolType::Kitty, c, r, 1);
    }
}
