//! `screen_snapshot` against real games, which is the only place its field list
//! can actually be wrong (SQ-1401).
//!
//! The in-crate cases (`zvm::screen_snapshot::tests`) build screens by hand and
//! so can only prove that what the encoder writes the decoder reads back. They
//! cannot prove that the encoder writes everything a real game PUT on the screen
//! — that is exactly the class of defect the host's mirrored DTOs kept meeting,
//! one field at a time, on one story at a time (SQ-0585, SQ-0749, SQ-0820).
//!
//! So each case here boots a commercial story, drives it far enough to have a
//! screen worth carrying, and compares the restored screen with the original
//! FIELD BY FIELD — after PERTURBING the machine in between, because a snapshot
//! compared against a machine that never moved passes even when the restore does
//! nothing at all.
//!
//! `stories/` is gitignored, so every case SKIPs vacuously when its fixture is
//! absent (CI has none). Each one therefore also asserts the shape it depends on
//! — an upper window with text in it, a v6 window table with paint in it — so a
//! story that boots differently fails here rather than passing on an empty
//! screen.

use std::path::PathBuf;

use zvm::cpu::boot::BootConfig;
use zvm::cpu::exec::{Machine, StepResult};
use zvm::error::ZError;
use zvm::memory::Memory;
use zvm::screen::{ScreenState, UpperWindow};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

const MAX_STEPS: u64 = 50_000_000;

/// Boot `file` and run to its first input request, or `None` when the gitignored
/// story is absent.
///
/// `honor_game_colours` is on: a story that cannot see the colour bit never calls
/// `set_colour`, and a snapshot of a colourless screen cannot fail on a colour
/// field.
fn boot(file: &str) -> Option<Machine> {
    let path = stories_dir().join(file);
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mem = Memory::new(raw).expect("a valid story file");
    let mut m = Machine::boot(
        mem,
        Box::new(zvm::io::BufferOutput::new()),
        BootConfig::new().with_honor_game_colours(true).with_rng_seed(1),
    );
    run_to_input(&mut m);
    Some(m)
}

fn run_to_input(m: &mut Machine) {
    let mut steps = 0u64;
    loop {
        match m.step() {
            StepResult::NeedLine { .. } | StepResult::NeedChar => return,
            StepResult::Continue => {
                steps += 1;
                assert!(steps < MAX_STEPS, "runaway: no input request within {MAX_STEPS} steps");
            }
            StepResult::Quit => return,
            StepResult::Fault => panic!("faulted: {:?}", m.take_fault_trace()),
            other => panic!("unexpected signal before input: {other:?}"),
        }
    }
}

/// Play one turn, so the screen the restore has to overwrite is a DIFFERENT one.
fn play(m: &mut Machine, line: &str) {
    m.supply_line(line, 13);
    run_to_input(m);
}

// ---------------------------------------------------------------------------
// Field-by-field comparison
// ---------------------------------------------------------------------------

/// Everything the blob is documented to carry, named one field at a time so a
/// failure says WHICH field the encoder dropped.
fn assert_same(a: &ScreenState, b: &ScreenState) {
    assert_eq!(a.upper_window_rows, b.upper_window_rows, "upper_window_rows");
    assert_eq!(a.current_window, b.current_window, "current_window");
    assert_eq!(a.text_style, b.text_style, "text_style");
    assert_eq!(a.cursor_row, b.cursor_row, "cursor_row");
    assert_eq!(a.cursor_col, b.cursor_col, "cursor_col");
    assert_eq!(a.buffer_mode, b.buffer_mode, "buffer_mode");
    assert_eq!(a.show_status_requested, b.show_status_requested, "show_status_requested");
    assert_eq!(a.v6_input_window, b.v6_input_window, "v6_input_window");
    assert_grid_same(&a.upper, &b.upper, "upper window");
    match (&a.v6, &b.v6) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            assert_eq!(x.current, y.current, "v6.current");
            for (i, (wx, wy)) in x.windows.iter().zip(y.windows.iter()).enumerate() {
                for n in 0..16u16 {
                    assert_eq!(wx.get_prop(n), wy.get_prop(n), "window {i} property {n}");
                }
                assert_eq!((wx.fg, wx.bg), (wy.fg, wy.bg), "window {i} colours");
                assert_grid_same(&wx.grid, &wy.grid, &format!("window {i}"));
                assert_eq!(wx.texts, wy.texts, "window {i} painted runs");
                assert_eq!(wx.prose, wy.prose, "window {i} prose");
                assert_eq!(wx.streamed, wy.streamed, "window {i} streamed runs");
                assert_eq!(wx.retired, wy.retired, "window {i} retired runs");
            }
        }
        _ => panic!("one screen has a v6 window table and the other does not"),
    }
}

fn assert_grid_same(a: &UpperWindow, b: &UpperWindow, what: &str) {
    assert_eq!(a.cols, b.cols, "{what} grid cols");
    assert_eq!(a.rows, b.rows, "{what} grid rows");
    assert_eq!(a.cells.len(), b.cells.len(), "{what} grid cell count");
    for (i, (x, y)) in a.cells.iter().zip(b.cells.iter()).enumerate() {
        assert_eq!((x.ch, x.style, x.fg, x.bg), (y.ch, y.style, y.fg, y.bg), "{what} cell {i}");
    }
}

fn grid_text(g: &UpperWindow) -> String {
    g.cells.iter().map(|c| c.ch).collect()
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// Beyond Zork is the Version 5 specimen because it reads `$2C`/`$2D` while
/// booting and picks a colour scheme from them, so its upper window is both
/// SPLIT and COLOURED — the two halves of a v4+ screen a snapshot has to carry.
#[test]
fn a_v5_upper_window_survives_a_snapshot_and_a_played_turn() {
    let Some(mut m) = boot("beyondzork-r57-s871221.z5") else { return };

    // The turns that reach a screen worth carrying, and what each answers:
    // "Is this a VT220?" → no; "BEGIN, RESTORE or QUIT?" → begin; then a blank
    // line past the title card into Character Setup. Named here because a frame
    // is a fixture: change the count and you are measuring a different screen.
    for line in ["no", "begin", ""] {
        play(&mut m, line);
    }

    assert!(m.screen.upper_window_rows > 0, "Beyond Zork has split an upper window by now");
    assert!(
        grid_text(&m.screen.upper).contains("B  E  Y  O  N  D"),
        "the upper window holds the title card: {:?}",
        grid_text(&m.screen.upper)
    );
    assert!(
        grid_text(&m.screen.upper).trim().len() > 4,
        "the upper window has text in it: {:?}",
        grid_text(&m.screen.upper)
    );
    assert_ne!(
        m.screen.current_fg,
        zvm::screen::ZColour::Default,
        "Beyond Zork read $2C/$2D while booting and chose a colour scheme"
    );

    let before = m.screen.clone();
    let blob = m.screen_snapshot();

    // PERTURB: the next turn re-splits to 14 rows and repaints the whole window
    // with the Character Setup menu, so a restore that did nothing would now be
    // comparing two different screens.
    play(&mut m, "look");
    assert_ne!(
        (grid_text(&m.screen.upper), m.screen.upper_window_rows),
        (grid_text(&before.upper), before.upper_window_rows),
        "the turn actually changed the screen (otherwise this proves nothing)"
    );

    m.restore_screen_snapshot(&blob).expect("the snapshot decodes");
    assert_same(&before, &m.screen);
}

/// Zork Zero is the Version 6 specimen: eight windows, a painted banner with
/// pixel-positioned runs on it, and per-window colours.
#[test]
fn a_v6_window_table_survives_a_snapshot_and_several_played_turns() {
    let Some(mut m) = boot("zork0-r393-s890714.z6") else { return };

    // One move off the opening banquet, into the Scullery. The turn count is
    // painted into window 1 and changes every turn, which is what makes the
    // perturbation below visible (a frame is a fixture — this one is turn 1).
    play(&mut m, "ne");

    let v6 = m.screen.v6.as_ref().expect("a v6 story has a window table");
    assert!(
        v6.windows[1].texts.iter().any(|r| r.text == "Scullery"),
        "window 1 carries the painted banner: {:?}",
        v6.windows[1].texts.iter().map(|r| &r.text).collect::<Vec<_>>()
    );
    assert!(
        v6.windows.iter().any(|w| w.x_size > 0 && w.y_size > 0),
        "…and the windows have a size"
    );

    let before = m.screen.clone();
    let blob = m.screen_snapshot();

    // PERTURB, as above: without a screen that has actually moved on, a restore
    // that did nothing at all would pass. Zork Zero repaints the turn counter
    // into window 1 on every turn, so one `wait` is enough.
    play(&mut m, "wait");
    let moved = {
        let (x, y) = (before.v6.as_ref().unwrap(), m.screen.v6.as_ref().unwrap());
        x.windows.iter().zip(y.windows.iter()).any(|(a, b)| a.texts != b.texts)
    };
    assert!(moved, "the turn actually changed the v6 screen (otherwise this proves nothing)");

    m.restore_screen_snapshot(&blob).expect("the snapshot decodes");
    assert_same(&before, &m.screen);
}

/// A snapshot the host cannot read must leave the machine alone rather than take
/// a screen away: Quetzal's own answer to "no screen" is that the story
/// repaints, and that is still available. Neither shape may panic.
#[test]
fn a_damaged_snapshot_is_refused_without_touching_the_machine() {
    let Some(mut m) = boot("beyondzork-r57-s871221.z5") else { return };
    let before = m.screen.clone();
    let blob = m.screen_snapshot();

    for n in [0usize, 3, 7, blob.len() / 2, blob.len() - 1] {
        assert_eq!(
            m.restore_screen_snapshot(&blob[..n]).unwrap_err(),
            ZError::BadScreenSnapshot,
            "truncated to {n} bytes"
        );
        assert_same(&before, &m.screen);
    }

    let mut newer = blob.clone();
    let bumped = zvm::screen_snapshot::VERSION + 1;
    newer[4..6].copy_from_slice(&bumped.to_be_bytes());
    assert_eq!(
        m.restore_screen_snapshot(&newer).unwrap_err(),
        ZError::ScreenSnapshotVersion { found: bumped, supported: zvm::screen_snapshot::VERSION }
    );
    assert_same(&before, &m.screen);
}
