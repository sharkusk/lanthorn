//! Where the `[more]` pager PARKS the view — SQ-0823.
//!
//! The arming rules are `more_pager_arming.rs`'s subject; this suite asks the one
//! question that comes after them, and that only a rendered frame can answer:
//! when the pager engages, is the FIRST new row the top row on screen?
//!
//! The report, playing Arthur: *"it scrolls one line too many before the [more]
//! prompt is shown"* — the pause comes a line late and a line of prose slips past.
//! It did, and the arithmetic was never the whole of it. `activation_target`
//! parked the view against the number the renderer called `viewport_rows`, and on
//! the frame the reader actually looks at, that number was not the count of rows
//! carrying prose:
//!
//! 1. the `[more]` bar takes a row out of the transcript for itself, and the park
//!    is computed one frame BEFORE it appears — so the row that would have been at
//!    the top is the row the bar displaces (Arthur, hybrid, shipped defaults);
//! 2. `viewport_rows` was the pane RECT, not the transcript body: a v3 status line
//!    (Cutthroats), the optional command bar, a suggestion strip. Each counted as a
//!    readable row, so a turn overflowing by exactly those rows raised no `[more]`
//!    at all and scrolled past in silence.
//!
//! Both are engine-neutral — the pager is shared by the Z-machine, Glulx and Scott
//! — so the cases below drive a v6 hybrid frame (Arthur, the reporter's game), the
//! Amiga release floppy of the same title (a different BUILD, release 54), the
//! same v6 game through the RASTER path (whose prompt is stamped over the last
//! prose row and so takes nothing — the fix must not move it), and a plain v3
//! story whose status line is the second defect in isolation.
//!
//! `stories/` is gitignored: a missing fixture skips vacuously, loudly on stderr.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way the app does (pictures resolved, boot plates flushed)
/// and tap through its intro to ordinary play.
/// `release`/`serial` pin the BUILD the medium carries (`real_media_releases.rs`
/// is the authority for the table), so a failure can never be attributed to the
/// wrong one — a disk image is a different release, not the same story on other
/// media.
fn boot_v6(file: &str, release: u16, serial: &str, honor: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_mounted_story(&path) {
        Ok((loaded, _)) => loaded.bytes().to_vec(),
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    assert_eq!(bytes[0], 6, "{file}: Z-machine version");
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file}: this medium carries a DIFFERENT build than the case says"
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s =
        GameSession::new_with_trace(bytes, honor, false, None, false, picture_dims, std_win, None, None)
            .expect("v6 story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    Some(s)
}

fn story_state(honor: bool) -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.config.honor_game_colours = honor;
    // The park is a scroll, and a scroll TWEENS: `effective_transcript_scroll`
    // reports a rounded intermediate while the ease is in flight, so a frame drawn
    // the instant after the park would be measured mid-animation, not at the
    // target. Every case here asks where the view SETTLES.
    st.config.animation.enabled = false;
    st
}

/// The outcome of one arm → frame → activate → frame cycle: what the pager did,
/// and what the frame the reader sees actually shows.
struct Parked {
    /// Absolute wrapped-row index of the first row this turn added.
    first_new_row: u16,
    /// Absolute wrapped-row index drawn at the TOP of the parked frame.
    top_visible_row: usize,
    /// Rows of prose the parked frame shows.
    viewport_rows: u16,
    /// Whether the `[more]` prompt is up.
    active: bool,
    /// Whether the parked frame really drew the `[more]` label.
    label_on_screen: bool,
    /// Every row of the parked frame as text, for cases that assert on the prose
    /// the reader is actually looking at rather than on a row index.
    screen: Vec<String>,
}

/// Fold one turn's output into the transcript exactly as the run loop does
/// (`turn.rs`): a menu redraw collapses the previous reprint back to its clear
/// anchor, a screen clear re-anchors without deleting scrollback, and a turn that
/// carries `TranscriptElem`s goes through them in order — which is the only way a
/// clear that lands in the MIDDLE of a turn's output reaches the transcript. A
/// hint/menu page is exactly that shape (Shogun's boot turn prints nine header
/// lines, moves window 0, then prints into the new box), so a harness that only
/// pushed `result.transcript` would measure a screen the app never draws.
fn apply_turn(state: &mut AppState, r: &app::session::TurnResult) {
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(&r.transcript, app::state::TranscriptKind::Story, &r.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
    }
}

/// The keypress half of `apply_game_driven_result` (`turn.rs`, which lives in the
/// binary): a `read_char` turn's output goes through the char-echo push, which
/// folds it onto the line the game's cursor was already on. Returns whether it
/// folded — the pager's baseline needs it (SQ-0823).
fn apply_keypress(state: &mut AppState, s: &GameSession, r: &app::session::TurnResult) -> bool {
    if !r.transcript_elems.is_empty() {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
        return false;
    }
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
        state.push_transcript_runs(&r.transcript, app::state::TranscriptKind::Story, &r.transcript_runs);
        return false;
    }
    state.push_transcript_runs_char_echo(
        &r.transcript,
        app::state::TranscriptKind::Story,
        &r.transcript_runs,
        s.output_continued_line(),
    )
}

/// Drive `cmds` as one turn's worth of output and run the run loop's own pager
/// sequence around it: cache the pre-turn total, arm, render (the frame that
/// decides), `apply_frame`, render again (the frame the reader sees).
fn park(s: &mut GameSession, state: &mut AppState, area: Rect, cmds: &[&str]) -> Parked {
    park_with(s, state, area, |s, state| {
        for c in cmds {
            let t = s.submit(c);
            apply_turn(state, &t);
        }
        false
    })
}

/// [`park`] for a single `read_char` keypress — the turn kind Arthur's hint pages
/// are read with, and the only one whose output can land on a row that was already
/// on screen.
fn park_key(s: &mut GameSession, state: &mut AppState, area: Rect, key: u8) -> Parked {
    park_with(s, state, area, |s, state| {
        let t = s.submit_char(key);
        apply_keypress(state, s, &t)
    })
}

/// The arm → frame → activate → frame cycle shared by both. `drive` runs the turn
/// and reports whether its output CONTINUED the transcript's last pre-turn row, the
/// one case where that row is not wholly old (SQ-0823).
fn park_with(
    s: &mut GameSession,
    state: &mut AppState,
    area: Rect,
    drive: impl FnOnce(&mut GameSession, &mut AppState) -> bool,
) -> Parked {
    let render = |state: &AppState, s: &GameSession| {
        let mut buf = Buffer::empty(area);
        let m = app::render::screen::render_story_pane(&s.screen(), false, None, state, area, &mut buf);
        (m, state.transcript_geom.get(), buf)
    };

    // Flush anything the session buffered before this turn (boot output). Only when
    // there IS some: `push_transcript("")` pushes one empty line, and a blank line
    // dropped after the game's read prompt is a line the game never printed — it
    // takes the cursor's row out from under the turn about to continue it.
    let pending = s.take_transcript();
    if !pending.is_empty() {
        state.push_transcript(&pending);
    }
    let (m0, _, _) = render(state, s);
    state.last_transcript_total_rows = m0.total_rows;

    let continued_row = drive(s, state);
    // Rows `0..total` are old and row `total` is the first row this turn adds —
    // unless the turn continued the last of them, which makes that row partly its
    // own and so the first new row on screen. Spelled out rather than taken from
    // `app::pager::baseline_before`, so that reverting the fix moves what the run
    // loop arms with while leaving what the reader is owed where it is.
    let first_new_row = m0.total_rows - u16::from(continued_row && m0.total_rows > 0);
    state.pager.arm(app::pager::baseline_before(m0.total_rows, continued_row));

    let (m1, _, _) = render(state, s);
    app::pager::apply_frame(
        state,
        m1.max_scroll,
        m1.viewport_rows,
        m1.prompt_rows,
        m1.total_rows,
        m1.transcript_surface,
        m1.top_anchored_fits,
    );
    // The raster path builds its composite (and its metrics) on a worker thread and
    // redraws the last-ready one meanwhile; the run loop polls that job every tick,
    // so a harness that doesn't would measure the PREVIOUS frame's geometry.
    for _ in 0..400 {
        if state.poll_v6_encode_job() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (m2, g2, buf) = render(state, s);
    let g2 = g2.expect("the parked frame lays the transcript out");
    let screen: Vec<String> = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect();
    let label_on_screen = screen.iter().any(|row| row.contains("[more]"));
    Parked {
        first_new_row,
        top_visible_row: g2.first_abs_row,
        viewport_rows: m2.viewport_rows,
        active: state.pager.active,
        label_on_screen,
        screen,
    }
}

/// Assert the whole point: the pager engaged, and the reader's eye lands on the
/// first row of the new output with nothing of it above the fold.
///
/// `cell_prompt` says whether the `[more]` label should be findable as buffer TEXT
/// — true on the cell paths, false on the raster one, which blits it as glyph
/// pixels into the composite image where no cell carries it.
fn assert_nothing_scrolled_past(p: &Parked, cell_prompt: bool, ctx: &str) {
    assert!(p.active, "{ctx}: this turn overflows the pane — the pager must engage");
    assert_eq!(
        p.label_on_screen, cell_prompt,
        "{ctx}: an engaged pager draws its [more] prompt as terminal text on the cell paths only"
    );
    assert_eq!(
        p.top_visible_row, p.first_new_row as usize,
        "{ctx}: the FIRST new row must be the top row of the parked frame — it sits {} row(s) above \
         the fold, unread, which is the reported symptom (viewport {} rows)",
        p.top_visible_row as i64 - p.first_new_row as i64,
        p.viewport_rows,
    );
}

/// Arthur, hybrid, shipped defaults — the reported case, in both colour modes.
///
/// The `[more]` bar reserves the bottom row of the transcript for itself, and the
/// park is computed on the frame before it appears. Measured off
/// `arthur-r74-s890714.z6` at 80x30: 25 rows added into a 17-row viewport parked
/// at offset 8 and drew the first new row at absolute row 2 — one past the row the
/// turn started on. It parks at 9 now.
#[test]
fn arthur_hybrid_parks_the_first_new_row_at_the_top() {
    for honor in [true, false] {
        let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", honor) else { return };
        let mut state = story_state(honor);
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look", "look"]);
        assert_nothing_scrolled_past(&p, true, &format!("arthur-r74-s890714.z6 hybrid, honor={honor}"));
    }
}

/// The same title on its Amiga release floppy — a different BUILD (release 54 /
/// serial 890606, against the story file's release 74), so the report is answered
/// on the medium as well as on the bare file.
#[test]
fn arthur_amiga_floppy_parks_the_first_new_row_at_the_top() {
    let file = "Arthur - The Quest for Excalibur.adf";
    let Some(mut s) = boot_v6(file, 54, "890606", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, &format!("{file} [release 54, serial 890606] hybrid"));
}

/// The RASTER path's `[more]` is stamped over the tail of the last prose row
/// rather than given a row of its own, so its viewport does not shrink when the
/// prompt shows — `prompt_rows` is 0 there and the park must stay where it was.
/// (The same fix applied blindly to both paths would push this one a row the wrong
/// way, showing a row of the PREVIOUS screen at the top instead.)
#[test]
fn arthur_raster_parks_the_first_new_row_at_the_top() {
    let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Raster;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look"]);
    assert_nothing_scrolled_past(&p, false, "arthur-r74-s890714.z6 raster");
}

/// The second half of the defect, with no `[more]` row involved at all: the pane
/// rect is not the transcript. Cutthroats is a v3 story, so its status line takes
/// the top row of the story pane — and the pager was handed the rect, status row
/// included. Measured at 80x30: a turn adding exactly 30 rows into a 29-row body
/// read as "fits, nothing missed", raised no prompt, and scrolled its first line
/// away.
#[test]
fn a_v3_status_line_is_not_a_readable_transcript_row() {
    let path = stories_dir().join("cutthroats-r23-s840809.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut s =
        GameSession::new_with_trace(bytes, true, false, None, false, Default::default(), None, None, None)
            .expect("Cutthroats boots");
    let mut state = story_state(true);
    let area = Rect::new(0, 0, 80, 30);

    // The status line's row is not a row the reader can read prose on.
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&s.screen(), false, None, &state, area, &mut buf);
    assert_eq!(
        m.viewport_rows,
        area.height - 1,
        "premise: the v3 status line takes one of the pane's {} rows",
        area.height
    );

    let p = park(&mut s, &mut state, area, &["verbose", "look", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, "cutthroats-r23-s840809.z3 (v3 status line)");
}

/// A turn that CLEARS in the middle of its own output still parks on the first new
/// row — the shape a hint or menu page has, and the one the arming ruleset's
/// SQ-0539 note is about ("a menu/hint page that CLEARS and then paints more than
/// a screenful").
///
/// DRIVEN SYNTHETICALLY since SQ-0895, and this is the second driver it has had.
/// The case was written on Shogun's boot turn (`shogun-r322-s890706.z6`), which
/// prints nine centred header lines, MOVES window 0 into a small box beside its
/// menu — landing a `TranscriptElem::ScreenClear` mid-output — then erases the new
/// box and prints into it. That turn no longer overflows any pane, so it can no
/// longer make the pager engage, and a park is only observable on an engaged pager.
/// Two merged changes took its bulk away, neither of them a defect:
///
/// 1. SQ-0890 retires those nine header lines as PAINT when window 0 walks away
///    from them, so they are not transcript at all any more;
/// 2. SQ-0461 used to echo Shogun's 320x200 title splash into the transcript as an
///    inline band for the frameless mode, upscaled 2x — roughly 25 of the 30 rows
///    that made the turn overflow. SQ-0895 removed the mode and the echo with it.
///
/// MEASURED after both, on the raster path at 80x30: `first_new_row = 0`,
/// `viewport_rows = 6`, four rows of menu, pager inactive. No pane size makes four
/// rows overflow while the frame still lays a transcript out, on any path.
///
/// So the property moves to a synthetic driver, exactly as
/// `a_continued_row_that_wraps_parks_on_the_row_it_shares_and_no_higher` already
/// does: a plain primary Buffer, a settled screen with scrollback and an anchor,
/// then one turn carrying `erase_lower` plus more rows than the viewport holds.
/// That is the shape the Shogun turn had, with nothing about a v6 render mode in
/// it — and it needs no gitignored fixture, so it stops skipping vacuously in CI.
#[test]
fn a_turn_that_clears_mid_output_parks_on_the_first_new_row() {
    use app::engine::{BufferWindow, ScreenModel, StatusModel, WinNode};

    for honor in [true, false] {
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut state = story_state(honor);
        let render = |state: &AppState| {
            let mut buf = Buffer::empty(area);
            let m = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
            (m, state.transcript_geom.get())
        };

        // A settled screen with scrollback behind it — the state the app is in when
        // a game opens a menu page over what the player was reading.
        for i in 0..20 {
            state.push_transcript(&format!("old line {i}"));
        }
        let (m0, _) = render(&state);
        state.last_transcript_total_rows = m0.total_rows;

        // The turn: it CLEARS mid-output (re-anchoring, scrollback preserved) and
        // then prints far more rows than the 12-row pane can hold.
        let r = app::session::TurnResult {
            erase_lower: true,
            transcript: (0..20).map(|i| format!("new line {i}")).collect::<Vec<_>>().join("\n"),
            ..Default::default()
        };
        apply_turn(&mut state, &r);
        let first_new_row = state.clear_anchor.expect("the mid-output clear re-anchored") as u16;
        state.pager.arm(app::pager::baseline_before(first_new_row, false));

        let (m1, _) = render(&state);
        app::pager::apply_frame(
            &mut state, m1.max_scroll, m1.viewport_rows, m1.prompt_rows, m1.total_rows, m1.transcript_surface,
            m1.top_anchored_fits,
        );
        let (_, g2) = render(&state);
        let g2 = g2.expect("the parked frame lays the transcript out");

        let ctx = format!("synthetic mid-output clear, honor={honor}");
        assert!(state.pager.active, "{ctx}: this turn overflows the viewport — the pager must engage");
        assert_eq!(
            g2.first_abs_row, first_new_row as usize,
            "{ctx}: the FIRST new row must be the top row of the parked frame — it sits {} row(s) \
             above the fold, unread (viewport {} rows)",
            g2.first_abs_row as i64 - first_new_row as i64,
            m1.viewport_rows,
        );
    }
}

/// A turn whose output CONTINUES the row the game left the cursor on — the report,
/// exactly as the user played it.
///
/// Arthur's InvisiClues print a `1> ` prompt and wait on a key; the key that
/// answers prints the page AFTER that prompt, on that row, and the char-echo push
/// folds it there (`push_transcript_runs_char_echo`, SQ-0804) because that is where
/// the game's own cursor is. The pre-turn row is therefore partly this turn's, and
/// the baseline has to step back onto it — rows are atomic to `activation_target`,
/// so counting it as old parks one row low and the page's first line, `1> SENIOR
/// PROGRAMMER`, goes past the fold. Measured on the Amiga floppy at 80x48 before
/// the fix: `before_rows` 79, 124 rows after, parked with absolute row 79 (`Duane
/// Beck`) on top and its section heading gone.
///
/// Driven from a cold boot rather than from a save, so it needs nothing but the
/// medium: `get torque`, `examine crystal` twice to open the crystal, then the
/// InvisiClues keys — main menu, down to NOTES, into it, down to Credits, and the
/// two returns that select the page and turn it.
#[test]
fn a_turn_that_continues_the_row_above_it_parks_on_that_row() {
    let file = "Arthur - The Quest for Excalibur.adf";
    for honor in [true, false] {
        for rows in [30, 48] {
            let Some(mut s) = boot_v6(file, 54, "890606", honor) else { return };
            let mut state = story_state(honor);
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.config.v6_render = app::config::V6RenderMode::Hybrid;
            // Inline-prompt mode, the shipped default: `startup.rs` hands
            // `command_bar` (false) straight to `set_strip_prompt`, so the game's own
            // read prompt stays in the transcript. That prompt IS the row this case
            // is about.
            s.set_strip_prompt(false);
            let area = Rect::new(0, 0, 80, rows);
            let ctx = format!("{file} [release 54, serial 890606] InvisiClues, honor={honor}, {rows} rows");

            for cmd in ["get torque", "examine crystal", "examine crystal"] {
                let t = s.submit(cmd);
                apply_turn(&mut state, &t);
            }
            // m: main menu. n n n: down to NOTES. Return: into it. n x6: down to
            // Credits. Return: select it, which prints the `1> ` page prompt.
            for key in [b'm', b'n', b'n', b'n', 13, b'n', b'n', b'n', b'n', b'n', b'n', 13] {
                let t = s.submit_char(key);
                apply_keypress(&mut state, &s, &t);
            }
            assert_eq!(
                state.transcript.last().map(String::as_str),
                Some("1> "),
                "{ctx}: premise — the page prompt is the transcript's last line, unterminated"
            );
            assert_eq!(s.pending_input(), InputKind::Char, "{ctx}: premise — it is read with a key");

            // …and the key that turns the page prints onto that very row.
            let p = park_key(&mut s, &mut state, area, 13);
            assert!(
                state.transcript.iter().any(|l| l.starts_with("1> SENIOR PROGRAMMER")),
                "{ctx}: premise — the page's first line lands on the prompt's row"
            );
            assert_nothing_scrolled_past(&p, true, &ctx);
            assert!(
                p.screen.iter().any(|row| row.contains("SENIOR PROGRAMMER")),
                "{ctx}: the page's own first line must be ON the parked screen — it is the heading \
                 the names under it belong to, and the report is that it scrolls past unread:\n{}",
                p.screen.join("\n"),
            );
        }
    }
}

/// The boundary case in that arithmetic, with no story file involved: a continued
/// row that WRAPS.
///
/// The pager is shared by every engine, and its subject is RENDERED rows, not
/// logical lines — so this drives a bare `ScreenModel` with nothing but a primary
/// buffer in it. When the appended text turns the one row the prompt sat on into
/// three, two of those rows are wholly new and only the first is shared. The step
/// back is therefore one row and not three: counting the whole logical line as new
/// would park two rows of this turn's own output above the fold, the same defect
/// pointing the other way.
#[test]
fn a_continued_row_that_wraps_parks_on_the_row_it_shares_and_no_higher() {
    use app::engine::{BufferWindow, ScreenModel, StatusModel, WinNode};

    for honor in [true, false] {
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut state = story_state(honor);
        let render = |state: &AppState| {
            let mut buf = Buffer::empty(area);
            let m = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
            (m, state.transcript_geom.get())
        };

        // A settled screen whose last line is an unterminated prompt on a row of
        // its own — the shape a game leaves when it stops to read a key.
        for _ in 0..6 {
            state.push_transcript("filler");
        }
        state.push_transcript_runs("1> ", app::state::TranscriptKind::Story, &[]);
        let (m0, _) = render(&state);
        let prompt_row = m0.total_rows - 1;

        // The key that answers prints a long first line onto that very row, then a
        // screenful more below it.
        let mut page = "x".repeat(90);
        for i in 0..20 {
            page.push_str(&format!("\nline {i}"));
        }
        let folded = state.push_transcript_runs_char_echo(
            &page,
            app::state::TranscriptKind::Story,
            &[],
            true,
        );
        assert!(folded, "honor={honor}: premise — the push folds onto the prompt's line");
        assert!(
            state.transcript[6].starts_with("1> xxx"),
            "honor={honor}: premise — the page's first line joined the prompt, got {:?}",
            state.transcript[6]
        );

        let (m1, _) = render(&state);
        assert!(
            m1.total_rows >= prompt_row + 3,
            "honor={honor}: premise — the shared row wrapped into more than one row"
        );
        state.pager.arm(app::pager::baseline_before(m0.total_rows, folded));
        app::pager::apply_frame(
            &mut state,
            m1.max_scroll,
            m1.viewport_rows,
            m1.prompt_rows,
            m1.total_rows,
            m1.transcript_surface,
            m1.top_anchored_fits,
        );
        assert!(state.pager.active, "honor={honor}: the page overflows a 12-row pane");
        let (_, g2) = render(&state);
        assert_eq!(
            g2.expect("the parked frame lays the transcript out").first_abs_row,
            prompt_row as usize,
            "honor={honor}: the park lands on the ONE row the turn shares with what came before — \
             not on the logical line's first row, and not a row below it"
        );
    }
}

/// …and so is the optional command bar. With `command_bar = true` the live input
/// gets its own bottom row, which the pager also counted as readable — a second
/// row of prose past the fold for anyone running that config.
#[test]
fn the_command_bar_row_is_not_a_readable_transcript_row() {
    let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.command_bar = true;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, "arthur-r74-s890714.z6 hybrid + command_bar");
}
