//! SQ-1712 / SQ-1713: a host Restore State must put back what the NON-primary Glk
//! windows held (Narcolepsy's speech bubble, text window 2) and must keep the
//! scrollback the game cleared hidden (the pre-wake dream text).
//!
//! Before: gvm closes and reopens every backend window on `restore_state`, nothing
//! carried their contents, so the bubble stayed empty until the next turn
//! reprinted it (SQ-1712); and `apply_archive_state` reset the primary's clear
//! anchor, so the dream text the game had wiped reappeared in the left pane
//! (SQ-1713).
//!
//! Every case perturbs before asserting (wake, move, save/restore), reads the state
//! BEFORE any further turn runs, then delivers a non-reprinting resize and reads it
//! again. Input counts are named in each case. Skips vacuously without
//! `narco.blorb`.

use app::engine::{Engine, KeyInput, WinKind, WinNode};
use app::host::persist::{resume_from_turn, restore_file, save_state_now, ExitSave};
use app::host::screen::{glk_primary_text_window, resize_glulx};
use app::host::{boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

const STORY: &str = "narco.blorb";
/// A pane tall enough that, without the clear anchor, the dream text above the
/// wake-up is on screen (verified by falsification, see the commit).
const PANE: (u16, u16) = (160, 100);
const SMALL_PANE: (u16, u16) = (100, 60);

fn story() -> Option<std::path::PathBuf> {
    let p = fixture_path(STORY);
    if !p.is_file() {
        eprintln!("SKIP: {STORY} missing at {}", p.display());
        return None;
    }
    Some(p)
}

fn boot_with(path: std::path::PathBuf, home: &std::path::Path, fresh: bool, history: bool) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: path,
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.to_path_buf(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            enable_sound: false,
            auto_save: false,
            record_turn_history: history,
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: fresh,
    };
    boot_story(req, &mut QuietBoot).expect("narcolepsy boots headlessly")
}

fn boot(path: std::path::PathBuf, home: &std::path::Path) -> BootedStory {
    boot_with(path, home, true, false)
}

fn finish(b: &mut BootedStory, cmd: &str, newline: bool, result: app::session::TurnResult) {
    let mut tidy = 0u32;
    let _ = finish_command_turn(cmd, newline, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy);
}

fn play(b: &mut BootedStory, cmd: &str) {
    let result = b.session.submit(cmd);
    finish(b, cmd, true, result);
}

fn key(b: &mut BootedStory) {
    let result = b.session.submit_key(KeyInput::Char(' ')).expect("a key turn");
    finish(b, "", false, result);
}

/// The text of every non-primary text buffer in the window tree, `id:lines`.
fn bubble(b: &BootedStory) -> String {
    fn walk(n: &WinNode, out: &mut Vec<String>) {
        match n {
            WinNode::Buffer(w) if !w.primary => {
                let text: Vec<&str> = w.lines.iter().map(String::as_str).filter(|l| !l.is_empty()).collect();
                if !text.is_empty() {
                    out.push(format!("{}:{}", w.win, text.join("|")));
                }
            }
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    walk(&b.session.screen().root, &mut v);
    v.join(" ## ")
}

fn render(b: &mut BootedStory, (cols, rows): (u16, u16)) -> (Buffer, Vec<(u32, WinKind, Rect)>) {
    assert!(resize_glulx(b.session.as_mut(), (cols, rows)), "a Glulx session");
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let model = b.session.screen();
    let wins = app::render::screen::render_story_pane(&model, false, None, &b.state, area, &mut buf).win_rects;
    (buf, wins)
}

fn row_text(buf: &Buffer, r: Rect, y: u16) -> String {
    (r.x..r.x + r.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()
}

/// The rendered rows of the primary text window, top to bottom.
fn primary_rows(b: &mut BootedStory, pane: (u16, u16)) -> Vec<String> {
    let id = glk_primary_text_window(b.session.as_mut()).expect("a primary text window");
    let (buf, wins) = render(b, pane);
    let r = wins
        .iter()
        .find(|(wid, kind, _)| *wid == id && *kind == WinKind::Buffer)
        .map(|(_, _, r)| *r)
        .expect("the primary window is drawn");
    (r.y..r.y + r.height).map(|y| row_text(&buf, r, y)).collect()
}

/// Wake up, then press a key: the game clears the primary and the bubble appears.
/// 2 inputs (one line, one key).
fn wake(b: &mut BootedStory) {
    play(b, "wake up");
    key(b);
    assert!(bubble(b).contains("Exits: well, this is my house"), "premise: the bubble is up: {}", bubble(b));
    assert!(b.state.clear_anchor.is_some(), "premise: the game cleared the primary on waking");
}

/// The left pane shows the post-wake screen, not the dream text above it.
fn assert_left_pane(label: &str, b: &mut BootedStory, pane: (u16, u16)) {
    let rows = primary_rows(b, pane);
    let first = rows.iter().find(|r| !r.trim().is_empty()).cloned().unwrap_or_default();
    assert_eq!(first, "Living room (on the futon)", "{label}: first rendered row of the primary at {pane:?}: {rows:#?}");
    let all = rows.join("\n");
    assert!(!all.contains("floor again") && !all.contains("Awake"), "{label}: dream text revived at {pane:?}:\n{all}");
}

/// `(window id, undrawn)` for every graphics window in the tree (SQ-1711's
/// pristine-canvas flag): a restored window the game had painted must not read as
/// undrawn, and one it had not must stay so.
fn graphics_undrawn(b: &BootedStory) -> Vec<(u32, bool)> {
    fn walk(n: &WinNode, out: &mut Vec<(u32, bool)>) {
        match n {
            WinNode::Graphics(g) => out.push((g.win, g.undrawn)),
            WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    walk(&b.session.screen().root, &mut v);
    v.sort();
    v
}

/// The bubble is `want`, in the model and on the rendered screen, now and after a
/// resize whose Arrange reprints nothing.
fn assert_bubble_and_pane(label: &str, b: &mut BootedStory, want: &str) {
    assert_eq!(bubble(b), want, "{label}: window 2 straight after the restore, before any turn");
    assert_left_pane(label, b, PANE);
    assert_eq!(bubble(b), want, "{label}: after rendering at {PANE:?}");
    let (buf, _) = render(b, SMALL_PANE);
    let screen: String = (0..SMALL_PANE.1).map(|y| row_text(&buf, Rect::new(0, 0, SMALL_PANE.0, SMALL_PANE.1), y)).collect::<Vec<_>>().join("\n");
    assert!(screen.contains("Exits:"), "{label}: the bubble is drawn at {SMALL_PANE:?}:\n{screen}");
    assert_eq!(bubble(b), want, "{label}: after a resize to {SMALL_PANE:?}");
    assert_left_pane(label, b, SMALL_PANE);
}

#[test]
fn host_restore_in_the_same_session_brings_the_bubble_and_the_clear_back() {
    let Some(path) = story() else { return };
    let home = app::scratch_dir("sq1712-same");
    let mut b = boot(path, &home);
    wake(&mut b); // 2 inputs
    assert_left_pane("before saving", &mut b, PANE);
    let saved = bubble(&b);
    let gfx = graphics_undrawn(&b);
    assert!(gfx.iter().any(|g| g.1) && gfx.iter().any(|g| !g.1), "non-vacuity: some graphics windows drawn, some not: {gfx:?}");
    let slot = home.join("slot.lanthorn");
    assert!(matches!(save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, &slot), ExitSave::Saved));

    play(&mut b, "go to kitchen"); // 3rd input: the bubble changes
    assert_ne!(bubble(&b), saved, "premise: moving changed window 2");

    restore_file(&mut *b.session, &mut b.mapper, &mut b.state, &slot, None).expect("restore");
    assert_eq!(graphics_undrawn(&b), gfx, "drawn windows stay drawn, undrawn ones stay undrawn");
    assert_bubble_and_pane("same session", &mut b, &saved);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn host_restore_into_a_fresh_session_and_a_different_pane_size() {
    let Some(path) = story() else { return };
    let home = app::scratch_dir("sq1712-fresh-src");
    let mut src = boot(path.clone(), &home);
    wake(&mut src); // 2 inputs
    let saved = bubble(&src);
    let slot = home.join("slot.lanthorn");
    assert!(matches!(save_state_now(&mut *src.session, &src.mapper, &src.state, &src.ifid, &slot), ExitSave::Saved));

    // A brand-new session at its first prompt (0 inputs), laid out at a
    // different pane size than the one the save was taken at.
    let home2 = app::scratch_dir("sq1712-fresh-dst");
    let mut dst = boot(path, &home2);
    assert_ne!(bubble(&dst), saved, "premise: the fresh session is not already showing the saved bubble");
    let _ = render(&mut dst, SMALL_PANE);
    restore_file(&mut *dst.session, &mut dst.mapper, &mut dst.state, &slot, None).expect("restore");
    assert_bubble_and_pane("fresh session", &mut dst, &saved);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&home2);
}

#[test]
fn rewind_brings_the_bubble_and_the_clear_back() {
    let Some(path) = story() else { return };
    let home = app::scratch_dir("sq1712-rewind");
    let mut b = boot_with(path, &home, true, true);
    wake(&mut b); // 2 inputs
    let saved = bubble(&b);
    let at = b.state.history.len() - 1; // the key turn
    play(&mut b, "go to kitchen"); // 3rd input
    assert_ne!(bubble(&b), saved, "premise: moving changed window 2");
    assert_eq!(b.state.history.len(), at + 2, "premise: three turns recorded");

    let out = resume_from_turn(&mut *b.session, &mut b.mapper, &mut b.state, at, None);
    assert!(out.ok, "rewind succeeds");
    assert_bubble_and_pane("rewind", &mut b, &saved);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn auto_resume_at_launch_brings_the_bubble_and_the_clear_back() {
    let Some(path) = story() else { return };
    let home = app::scratch_dir("sq1712-resume");
    let mut b = boot(path.clone(), &home);
    wake(&mut b); // 2 inputs
    let saved = bubble(&b);
    assert!(matches!(save_state_now(&mut *b.session, &b.mapper, &b.state, &b.ifid, &b.arc_file), ExitSave::Saved));
    drop(b);

    let mut again = boot_with(path, &home, false, false);
    assert!(again.resumed, "premise: the second boot resumed from the auto-save");
    assert_bubble_and_pane("auto-resume", &mut again, &saved);
    let _ = std::fs::remove_dir_all(&home);
}

/// The IFF chunks of a `FORM IFZS` blob as `(id, bytes-including-header-and-pad)`.
fn chunks(form: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= form.len() {
        let len = u32::from_be_bytes([form[i + 4], form[i + 5], form[i + 6], form[i + 7]]) as usize;
        let end = (i + 8 + len + (len % 2)).min(form.len());
        out.push(([form[i], form[i + 1], form[i + 2], form[i + 3]], form[i..end].to_vec()));
        i = end;
    }
    out
}

fn has_chunk(form: &[u8]) -> bool {
    chunks(form).iter().any(|(id, _)| id == b"LtWc")
}

fn strip_chunk(form: &[u8]) -> Vec<u8> {
    let mut body = b"IFZS".to_vec();
    for (id, bytes) in chunks(form) {
        if &id != b"LtWc" {
            body.extend_from_slice(&bytes);
        }
    }
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[test]
fn a_snapshot_without_the_window_chunk_still_restores() {
    // An older snapshot (no `LtWc` chunk) behaves as before: it restores, the
    // bubble is simply blank until the game reprints it.
    let Some(path) = story() else { return };
    let home = app::scratch_dir("sq1712-old");
    let mut b = boot(path, &home);
    wake(&mut b); // 2 inputs
    let saved = b.session.save_state();
    assert!(has_chunk(&saved.bytes), "premise: the current snapshot carries the chunk");
    let stripped = strip_chunk(&saved.bytes);
    assert!(!has_chunk(&stripped), "premise: the chunk is gone");
    let old = app::engine::EngineSave::new(saved.engine.clone(), saved.format_version, stripped);
    play(&mut b, "go to kitchen"); // 3rd input
    b.session.restore_state(&old).expect("an older snapshot still loads");
    assert_eq!(bubble(&b), "", "no chunk, no reinstated bubble (the pre-SQ-1712 behaviour)");
    let _ = std::fs::remove_dir_all(&home);
}
