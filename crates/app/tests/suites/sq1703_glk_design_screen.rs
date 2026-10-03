//! SQ-1703 P2: a Glulx story laid out on a design-pixel Glk screen (the
//! Windows-Glk `.cfg` size) divides its splits in exact pixels, the way a GUI
//! Glk does, instead of in terminal cells.
//!
//! Specimens (both skip vacuously without `stories/`), booted through
//! `boot_story` the way startup boots, then switched with
//! `host::screen::set_glk_design_screen` and made borderless explicitly (P1
//! wires `WindowBorders=no` from the `.cfg`; this suite does not depend on it):
//!
//! * Photopia 2.01 (`photo201.blb`) at 640x480.
//! * Narcolepsy (`narco.blorb`) at 800x600.
//!
//! Every frame is measured at the first line-input prompt after `boot_story`,
//! with the number of inputs submitted to get there recorded in
//! [`INPUTS_TO_FIRST_PROMPT`] terms: 0 means no key or line was submitted.
//! The design cell is non-square (6.5 x 13 design px) so a square-cell
//! assumption cannot pass.

use std::path::PathBuf;

use app::config::Config;
use app::engine::{Engine, KeyInput};
use app::glulx_session::GlulxSession;
use app::host::screen::{glk_layout, set_glk_design_screen, GlkLayout, GlkScreen};
use app::host::{boot_story, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;
use gvm::glk::WinType;

use crate::fixture_paths::fixture_path;

fn story(name: &str) -> Option<PathBuf> {
    let p = fixture_path(name);
    if !p.is_file() {
        eprintln!("SKIP: {name} missing at {}", p.display());
        return None;
    }
    Some(p)
}

/// Boot the way startup does, borderless, in design mode. Returns the session
/// and the number of inputs submitted before the first line prompt.
fn boot_design(path: PathBuf, size: (u32, u32), cell: (f64, f64)) -> (Box<dyn Engine>, usize) {
    let home = app::scratch_dir("sq1703");
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
    let mut booted = boot_story(req, &mut QuietBoot).expect("boots headlessly");
    let gs = booted
        .session
        .as_any_mut()
        .downcast_mut::<GlulxSession>()
        .expect("a Glulx story");
    gs.set_borderless(true);
    assert!(set_glk_design_screen(booted.session.as_mut(), Some(GlkScreen::design(size, cell))));
    let mut inputs = 0;
    for _ in 0..40 {
        match Engine::pending_input(booted.session.as_ref()) {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = booted.session.submit_key(KeyInput::Char(' '));
                inputs += 1;
            }
            _ => break,
        }
    }
    assert_eq!(Engine::pending_input(booted.session.as_ref()), InputKind::Line, "reached a line prompt");
    (booted.session, inputs)
}

fn rects(l: &GlkLayout, ty: WinType) -> Vec<(u32, u32, u32, u32)> {
    let mut v: Vec<_> = l
        .windows
        .iter()
        .filter(|w| w.wintype == ty)
        .map(|w| (w.rect.left, w.rect.top, w.rect.width, w.rect.height))
        .collect();
    v.sort();
    v
}

#[test]
fn photopia_at_640x480_lays_out_in_exact_pixels() {
    let Some(path) = story("photo201.blb") else { return };
    let (mut sess, inputs) = boot_design(path, (640, 480), (6.5, 13.0));
    let l = glk_layout(sess.as_mut()).expect("a Glulx story has a Glk layout");
    // Measured with 0 inputs: the first line prompt.
    assert_eq!(inputs, 0, "premise: measured at the first line prompt, 0 inputs");
    assert_eq!(l.screen, GlkScreen::design((640, 480), (6.5, 13.0)));
    // Non-vacuity: the frame's shape — one story buffer and four frame canvases.
    assert_eq!(rects(&l, WinType::TextBuffer).len(), 1, "one story text window");
    assert_eq!(rects(&l, WinType::Graphics).len(), 4, "four frame graphics windows");
    assert_eq!(rects(&l, WinType::TextBuffer), vec![(14, 58, 612, 343)], "story text 612x343 at (14,58)");
    assert_eq!(
        rects(&l, WinType::Graphics),
        vec![(0, 0, 640, 58), (0, 58, 14, 343), (0, 401, 640, 79), (626, 58, 14, 343)],
        "frames 640x58, 14x343, 640x79, 14x343 in exact pixels"
    );
}

#[test]
fn narcolepsy_at_800x600_lays_out_in_exact_pixels() {
    let Some(path) = story("narco.blorb") else { return };
    let (mut sess, inputs) = boot_design(path, (800, 600), (6.5, 13.0));
    let l = glk_layout(sess.as_mut()).expect("a Glulx story has a Glk layout");
    assert_eq!(inputs, 0, "premise: measured at the first line prompt, 0 inputs");
    // Non-vacuity: two text windows (a left navy half and the story text) and
    // five graphics windows (title, four margins).
    assert_eq!(rects(&l, WinType::TextBuffer).len(), 2);
    assert_eq!(rects(&l, WinType::Graphics).len(), 5);
    assert!(
        rects(&l, WinType::TextBuffer).contains(&(480, 60, 240, 378)),
        "story text 240x378 at (480,60): {:?}",
        rects(&l, WinType::TextBuffer)
    );
    assert!(
        rects(&l, WinType::Graphics).contains(&(0, 0, 400, 80)),
        "title graphics 400x80: {:?}",
        rects(&l, WinType::Graphics)
    );
}

/// Leaving design mode returns to the cell layout (the unit is a terminal cell
/// again), and the existing cell-mode entry points are untouched by it.
#[test]
fn leaving_design_mode_restores_cell_layout() {
    let Some(path) = story("photo201.blb") else { return };
    let (mut sess, _) = boot_design(path, (640, 480), (6.5, 13.0));
    assert!(set_glk_design_screen(sess.as_mut(), None));
    let l = glk_layout(sess.as_mut()).unwrap();
    assert_eq!(l.screen.text_cell, (1.0, 1.0));
    assert_ne!(l.screen.size, (640, 480));
}
