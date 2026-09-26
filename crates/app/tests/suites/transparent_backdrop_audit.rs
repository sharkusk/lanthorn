//! SQ-0704 cross-engine audit: what does a graphics window's TRANSPARENT pixel
//! resolve to, on each engine?
//!
//! The Z-machine v6 answer is fixed in `v6_zork0_icon_backdrop.rs` (a chrome
//! window's unpainted area now takes that window's own page, ZMSD §8.8.3.2).
//! This file is the standing evidence for the other two engines, where the audit
//! found no defect — the assertions below are what makes "no change needed" a
//! measurement rather than an assumption, and they will fail the day a game or a
//! refactor introduces the exposure.
//!
//! **Glulx.** Glk §7.2 on `glk_window_set_background_color`: *"This sets the
//! window's background color. It does not change what is currently displayed; it
//! only affects subsequent clears and resizes."* `graphics::Canvas` implements
//! exactly that (`set_background` stores it; `erase_rect`/`resize` fill with it —
//! pinned by `graphics::tests::erase_uses_background_color`), so a Glk graphics
//! window that the game cleared is opaque in its own colour. Text windows never
//! expose the question at all: a Glk grid/buffer carries its own `bg`/`fg` and
//! renders as terminal CELLS, which always have a background.
//!
//! **Scott.** The room-picture band is a graphics window over the room panel.
//! Scott art is full-frame raster with no alpha channel at all, so there is no
//! transparent pixel to resolve.
//!
//! In both engines the window rect is also flooded with the themed `graphics`
//! style *before* the canvas is placed (`render::graphics::GraphicsRender::render`),
//! so even the aspect-preserving letterbox around an upscaled picture is a
//! deliberate, `style.toml`-selectable colour rather than an accidental black.
//!
//! Every case skips cleanly when its gitignored story is absent.

use app::engine::{Engine, GraphicsWindow, WinNode};

use crate::fixture_paths::fixture_path as stories;

fn graphics_windows(node: &WinNode, out: &mut Vec<GraphicsWindow>) {
    match node {
        WinNode::Graphics(g) => out.push(g.clone()),
        WinNode::Pair { first, second, .. } => {
            graphics_windows(first, out);
            graphics_windows(second, out);
        }
        WinNode::Layered(items) => {
            for it in items {
                graphics_windows(&it.node, out);
            }
        }
        _ => {}
    }
}

/// Every pixel of `canvas` carries alpha — nothing is left for a compositor.
fn assert_fully_opaque(gw: &GraphicsWindow, who: &str) {
    let clear = gw.canvas.pixels().filter(|p| p.0[3] == 0).count();
    assert_eq!(
        clear, 0,
        "{who} window {} ({}x{}) has {clear} transparent pixels — a backdrop the engine must resolve",
        gw.win,
        gw.canvas.width(),
        gw.canvas.height()
    );
}

#[test]
fn glulx_graphics_window_leaves_no_transparency_for_the_terminal() {
    let path = stories("advent.blb");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let blorb = blorb::Blorb::parse(raw.clone()).expect("advent.blb is a valid blorb");
    let app::hints::LoadedStory::Glulx(bytes) = app::hints::extract_story(raw).expect("extract") else {
        panic!("advent.blb holds a Glulx story");
    };
    let mut sess =
        app::glulx_session::GlulxSession::new(bytes, 138, 51, true, true, false, (8.0, 18.0), Some(blorb), &[])
            .expect("advent boots");
    let _ = sess.take_transcript();
    for _ in 0..4 {
        let _ = sess.submit("look");
        let _ = sess.take_transcript();
    }

    let model = sess.screen();
    let mut gws = Vec::new();
    graphics_windows(&model.root, &mut gws);
    assert!(!gws.is_empty(), "advent opens its clickable graphics toolbar");
    for gw in &gws {
        assert_fully_opaque(gw, "Glulx");
    }

    // The Glk text windows carry their own colour pair and render as cells, so
    // "what is behind them" is never a compositing question. advent sets none,
    // which is itself the point: the host theme governs, per window.
    fn text_colours(node: &WinNode, out: &mut Vec<(Option<u32>, Option<u32>)>) {
        match node {
            WinNode::Grid(g) => out.push((g.bg, g.fg)),
            WinNode::Buffer(b) => out.push((b.bg, b.fg)),
            WinNode::Pair { first, second, .. } => {
                text_colours(first, out);
                text_colours(second, out);
            }
            _ => {}
        }
    }
    let mut pairs = Vec::new();
    text_colours(&model.root, &mut pairs);
    assert!(!pairs.is_empty(), "advent opens a status grid and a story buffer");
}

#[test]
fn scott_room_picture_has_no_alpha_to_resolve() {
    let mut checked = 0usize;
    for name in ["golden_baton.blb", "time_machine.blb", "perseus_andromeda.blb"] {
        let path = stories(name);
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            continue;
        };
        let blorb = blorb::Blorb::parse(raw.clone()).ok();
        let app::hints::LoadedStory::Scott(bytes) = app::hints::extract_story(raw).expect("extract") else {
            panic!("{name} holds a Scott Adams story");
        };
        let mut sess = app::scott_session::ScottSession::new(bytes, blorb).expect("scott session");
        let _ = sess.take_transcript();
        let _ = sess.submit("look");

        let model = sess.screen();
        let mut gws = Vec::new();
        graphics_windows(&model.root, &mut gws);
        assert_eq!(gws.len(), 1, "{name} shows exactly one room-picture band");
        assert!(gws[0].upscale, "{name}'s room picture is upscaled into its band");
        assert_fully_opaque(&gws[0], name);
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no graphical Scott story present");
    }
}
