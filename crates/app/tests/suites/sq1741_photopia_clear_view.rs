//! SQ-1741: a Glulx story that wipes its page by CLOSING the story buffer window and
//! opening a fresh one (rather than calling `glk_window_clear`) must still act as a
//! screen clear. Photopia 2.01 (`stories/photo201.blb`, with its `.cfg` design beside
//! it) does exactly that between the red splash and the Mars scene: it closes the
//! car-scene buffer (window 12), opens and closes a graphics window for the splash,
//! then opens a new text buffer (window 16). No `window_clear` is ever called, so the
//! host never set the clear anchor and — on a pane taller than the transcript — the
//! old white car scene stayed on screen above the new red-on-black text.
//!
//! Turn sequence: "no" to the instructions question, then "wait" at each `>` prompt
//! and Space at each key pause / `[more]`, until the Mars scene
//! ("You are Wendy Mackaye, ...") is in the transcript. The settled frame is then
//! rendered with the scroll animation cleared.
//!
//! The pre-clear text keeps its own white background: scrolling back past the clear
//! shows it exactly as it was.
//!
//! Skips vacuously without the gitignored `stories/photo201.blb` (not on CI).

use app::engine::{Engine, KeyInput};
use app::host::{apply_game_driven_result, boot_story, finish_command_turn, BootRequest, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::pager::Driver;
use app::session::InputKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::fixture_paths::fixture_path;

struct Frames {
    settled: Buffer,
    scrolled_back: Buffer,
    w: u16,
    h: u16,
}

fn drive(w: u16, h: u16, honor: bool) -> Option<Frames> {
    let story = fixture_path("photo201.blb");
    if !story.is_file() {
        eprintln!("SKIP: no photo201.blb");
        return None;
    }
    let home = app::scratch_dir("sq1741-photopia");
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: story,
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.to_path_buf(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            enable_sound: false,
            honor_game_colours: honor,
            virtual_screen_cols: Some(w),
            virtual_screen_rows: Some(h),
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts { size: Some((w, h)), ..TerminalFacts::default() },
        fresh_start: true,
    };
    let mut b = boot_story(req, &mut QuietBoot).expect("photo201 boots");
    let area = Rect::new(0, 0, w, h);
    for step in 0..80 {
        let model = Engine::screen(&*b.session);
        let kind = Engine::pending_input(&*b.session);
        let char_mode = matches!(kind, InputKind::Char);
        let mut buf = Buffer::empty(area);
        let m = app::render::screen::render_story_pane(&model, char_mode, None, &b.state, area, &mut buf);
        app::pager::apply_frame(&mut b.state, m.max_scroll, m.viewport_rows, m.prompt_rows, m.total_rows, m.transcript_surface, m.top_anchored_fits);
        if b.state.transcript.iter().any(|l| l.starts_with("You are Wendy Mackaye")) {
            b.state.scroll_anim = None;
            let mut settled = Buffer::empty(area);
            let m = app::render::screen::render_story_pane(&model, char_mode, None, &b.state, area, &mut settled);
            // Scroll back a row at a time until the old page's last line is in view.
            let mut scrolled_back = Buffer::empty(area);
            for s in 1..=m.max_scroll {
                b.state.transcript_scroll = s;
                scrolled_back = Buffer::empty(area);
                app::render::screen::render_story_pane(&model, char_mode, None, &b.state, area, &mut scrolled_back);
                if (0..h).any(|y| text(&scrolled_back, w, y).contains("unmistakable red")) {
                    break;
                }
            }
            let _ = std::fs::remove_dir_all(&home);
            return Some(Frames { settled, scrolled_back, w, h });
        }
        if b.state.pager.active {
            let t = app::input::page_scroll(b.state.transcript_scroll, -1, m.viewport_rows, m.max_scroll);
            b.state.scroll_transcript_to(t);
            if t == 0 {
                b.state.pager.active = false;
            }
            continue;
        }
        match kind {
            InputKind::Char => {
                if let Some(r) = b.session.submit_key(KeyInput::Char(' ')) {
                    let _ = apply_game_driven_result(&mut b.state, &mut b.mapper, &r, &b.game_dir, None, &*b.session, Driver::PlayerInput);
                }
            }
            _ => {
                let cmd = if step == 0 { "no" } else { "wait" };
                let r = b.session.submit(cmd);
                let mut t = 0u32;
                let _ = finish_command_turn(cmd, true, r, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut t);
            }
        }
    }
    panic!("never reached the Mars scene");
}

fn text(buf: &Buffer, w: u16, y: u16) -> String {
    (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect::<String>().trim_end().to_string()
}

/// Column inside the story text where every row's background is sampled.
const SAMPLE_COL: u16 = 4;

fn check(w: u16, h: u16, honor: bool) {
    let Some(f) = drive(w, h, honor) else { return };
    let rows: Vec<String> = (0..f.h).map(|y| text(&f.settled, f.w, y)).collect();
    let wendy = rows
        .iter()
        .position(|r| r.contains("You are Wendy Mackaye"))
        .unwrap_or_else(|| panic!("{w}x{h} honor={honor}: non-vacuity — Mars scene must be on screen: {rows:#?}"));

    // (a) nothing from before the clear is in view.
    for r in &rows {
        for stale in ["Rob", "Time passes", "unmistakable red", ">wait"] {
            assert!(!r.contains(stale), "{w}x{h} honor={honor}: pre-clear text {stale:?} still in view: {rows:#?}");
        }
    }

    // (b) the blank rows above the first Mars line, and the empty area below the last
    // line, paint with the post-clear background (the Mars text's own).
    let bg_at = |y: usize| f.settled.cell((SAMPLE_COL, y as u16)).unwrap().bg;
    let mars_bg = bg_at(wendy);
    if honor {
        assert_eq!(mars_bg, Color::Rgb(0, 0, 0), "{w}x{h}: premise — Mars is black");
    }
    assert_ne!(mars_bg, Color::Rgb(255, 255, 255), "{w}x{h} honor={honor}: Mars rows must not be white");
    let last_text = rows.iter().rposition(|r| r.starts_with("   ") && !r.trim().is_empty()).unwrap();
    for y in wendy.saturating_sub(3)..wendy {
        assert_eq!(bg_at(y), mars_bg, "{w}x{h} honor={honor}: blank row {y} above the first Mars line has the wrong background");
    }
    // (a short pane has the decorative frame right under the prompt, so only tall panes)
    for y in (if h >= 50 { last_text + 1 } else { usize::MAX })..(last_text + 4).min(f.h as usize) {
        assert_eq!(bg_at(y), mars_bg, "{w}x{h} honor={honor}: empty row {y} below the last line has the wrong background");
    }

    // (c) the old page is still there in scrollback, untouched: white under honour.
    let back: Vec<String> = (0..f.h).map(|y| text(&f.scrolled_back, f.w, y)).collect();
    let (y, line) = back
        .iter()
        .enumerate()
        .find(|(_, r)| r.contains("Time passes") || r.contains("unmistakable red"))
        .unwrap_or_else(|| panic!("{w}x{h} honor={honor}: scrolling back must reveal the car scene: {back:#?}"));
    if honor {
        let col = line.find("Time passes").or_else(|| line.find("unmistakable red")).unwrap() as u16;
        assert_eq!(
            f.scrolled_back.cell((col, y as u16)).unwrap().bg,
            Color::Rgb(255, 255, 255),
            "{w}x{h}: a pre-clear line keeps its white background in scrollback"
        );
    }
}

#[test]
fn photopia_glulx_clear_starts_a_new_page_250x58_honoured() {
    check(250, 58, true);
}

#[test]
fn photopia_glulx_clear_starts_a_new_page_250x58_theme() {
    check(250, 58, false);
}

#[test]
fn photopia_glulx_clear_starts_a_new_page_120x40_honoured() {
    check(120, 40, true);
}

#[test]
fn photopia_glulx_clear_starts_a_new_page_120x40_theme() {
    check(120, 40, false);
}
