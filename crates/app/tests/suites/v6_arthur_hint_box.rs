//! SQ-1006 — Arthur's boxed messages arrive by `print_form`, and the box was empty.
//!
//! # The report, and why it pointed the wrong way
//!
//! A lane driving Arthur to its InvisiClues screen noticed in passing that on
//! the in-play `hint` turn the game's answer — *"If only you had a crystal
//! ball...."* — never reaches [`GameSession::submit`]'s returned transcript,
//! while ordinary turns do, and guessed the trigger was the `@window_size` /
//! `@scroll_window` pair that turn issues on window 0.
//!
//! Both halves are false, and the second falsifies itself: `look` on the turn
//! before issues `@window_size(win=0, y=192, x=584)` too and its prose reaches
//! the transcript intact. Nor is *"not in the transcript"* a defect — the
//! message is a BOX. Arthur lays window 3 across the bottom of the screen and
//! prints there, and window 3 is not a flowing-prose window (wrap set,
//! scrolling clear — ZMSD §8.8.3.1), so its text paints at pixels the way a
//! status line does instead of streaming to the host's scrollback. The
//! transcript is the right place for it not to be.
//!
//! # What was actually wrong
//!
//! The turn's real shape, from the engine's own screen trace:
//!
//! ```text
//! @output_stream(3, table=0x4125, width=0)   ; justify as if in window 0
//! print "If only you had a crystal ball...."  ; captured, not printed
//! @output_stream(-3)                          ; close: table now holds it
//! …lay window 3 out across the bottom, erase it, colour it…
//! @print_form(table=0x4125)                   ; ← EXT:0x1A, and a no-op stub
//! ```
//!
//! `print_form` was unimplemented, so the message reached neither the screen
//! nor anywhere else: it existed only as bytes in a table nothing read. And the
//! table it would have read was the wrong SHAPE — ZMSD §15 `output_stream` says
//! that with a width operand "the table will contain not ordinary text but
//! formatted text: see print_form", and §15 `print_form` says that is "a
//! sequence of lines, terminated with a zero word", where "each line is a word
//! containing the number of characters, followed by that many bytes". zvm wrote
//! the plain §7.1.2.1 layout instead — one count word and the text after it.
//!
//! Arthur reads that table itself, and the wrong shape was visible in its own
//! geometry: it sized the box for **six** lines (96 px) for a 34-character
//! message, because it walked one bogus record after another before stumbling
//! onto a zero word. With the formatted layout it asks for one line (16 px),
//! which is what a 272-px message in a 584-px box needs.
//!
//! # The specimen
//!
//! `stories/Arthur - The Quest for Excalibur.adf`, release **54** / serial
//! 890606 — the Amiga floppy, booted the way `startup.rs` boots (profile from
//! the medium, screen through `std_window → native_std_window →
//! profile.std_window`). The frame is **sixteen turns in**: fourteen blank
//! turns past the intro answering `n` to the restore question, then `look`
//! (which is what leaves window 0 at its full 192 px, so the box has somewhere
//! to come from), then `hint`.
//!
//! Skip-if-missing (gitignored media), and non-vacuous: a fixture that is
//! present but yielded no frame fails rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const FIXTURE: &str = "Arthur - The Quest for Excalibur.adf";
const RELEASE: u16 = 54;
const SERIAL: &str = "890606";

/// The game's own answer to `hint` in play, verbatim — four dots, not three.
const MESSAGE: &str = "If only you had a crystal ball....";

/// Panes swept. 80x25 is Arthur's own screen exactly (640x400 at an 8x16 cell);
/// 100x25 is the same height in a wider terminal, so the pair separates the
/// column mapping from the row mapping.
///
/// The three TALLER panes are SQ-1008, and they are the ones that were failing.
/// A pane with rows to spare puts the ring on a reclaiming plan (`frame` here —
/// Arthur's poles flank the story full height), which top-anchors the story and
/// grows its viewport into the letterbox slack. That growth used to run to
/// `area.bottom()` unconditionally: the viewport kept its native rect
/// `(28, 208, 584, 176)` = 11 native rows and came out 17 cells tall at 100x34,
/// 21 at 80x34 and 35 at 80x48, so it overdrew this box — and every other v6
/// window below the story window — at any terminal taller than the game's own
/// screen, which is most terminals. See [`the_box_survives_the_reclaim`] for the
/// shape that keeps them apart.
const PANES: &[(u16, u16)] = &[(80, 25), (100, 25), (100, 34), (80, 34), (80, 48)];

/// The story window's own native rect on this frame, and the box's — the two
/// things every geometry claim below is made of. Window 0 ends at native y 384
/// and window 3 is the 16-px row after it, the LAST text row of a 640x400
/// screen. That one row is what the reclaim used to swallow.
const NATIVE_STORY: (u16, u16, u16, u16) = (28, 208, 584, 176);
const NATIVE_BOX: (u16, u16, u16, u16) = (28, 384, 584, 16);
/// How many terminal rows the story viewport is worth at its native size — the
/// floor a reclaiming plan must stay above, since window 0 is a scrolling
/// buffer and the reclaimed rows are its history.
const NATIVE_STORY_ROWS: u16 = 11;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` boots — see the sibling suite's `boot` for why
/// every step of the chain matters (CLAUDE.md).
fn boot() -> Option<GameSession> {
    let path = stories_dir().join(FIXTURE);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "{FIXTURE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), SERIAL, "{FIXTURE}: serial");
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &boot)
    .unwrap_or_else(|e| panic!("{FIXTURE}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive to the in-play `hint` turn and hand back the session together with
/// that turn's transcript. See the module docs for the route.
fn hint_in_play() -> Option<(GameSession, String)> {
    let mut s = boot()?;
    for _ in 0..14 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        assert!(!s.quit, "{FIXTURE}: quit during the intro");
    }
    // `look` first: it is what leaves window 0 at its full height, so the turn
    // after it is a genuine "make room for a box" turn rather than a no-op.
    let r = s.submit("look");
    assert!(r.fault.is_none(), "{FIXTURE}: look faulted: {:?}", r.fault);
    assert!(
        r.transcript.contains("English churchyard"),
        "{FIXTURE}: the route must be standing in the churchyard, got {:?}",
        r.transcript
    );
    let r = s.submit("hint");
    assert!(r.fault.is_none(), "{FIXTURE}: hint faulted: {:?}", r.fault);
    Some((s, r.transcript))
}

/// The bottom box as the app publishes it: the one-row Grid the layered v6
/// model carries for window 3, with the pixel-positioned runs the game painted
/// into it.
fn box_runs(s: &GameSession) -> Vec<String> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("{FIXTURE}: a v6 frame is Layered") };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The pane's rows as chars — searched row by row rather than as one joined
/// string, because a kitty placeholder is four bytes and a byte offset is not a
/// column.
fn rows(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')))
                .collect()
        })
        .collect()
}

fn state(honor: bool) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(app::render::graphics::kitty_picker(8, 16));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = honor;
    st
}

/// The cell rect the ring gave the story viewport on the frame just rendered,
/// read back out of `v6_cell_map` — the same record `/dump-windows` reports, so
/// a claim made here is a claim about what the player's screen was carved into.
fn viewport_cells(st: &app::state::AppState) -> (u16, u16, u16, u16) {
    st.v6_cell_map
        .borrow()
        .iter()
        .find(|c| c.label == "viewport")
        .map(|c| {
            assert_eq!(c.native, NATIVE_STORY, "{FIXTURE}: the viewport's native rect");
            c.cells
        })
        .expect("a hybrid-ring frame records its story viewport")
}

/// **The report.** The message the game answers `hint` with reaches the screen:
/// the app publishes it as a painted run in the bottom box, and hybrid draws it
/// with glyphs at every pane size.
#[test]
fn the_hint_box_carries_the_games_answer() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, _)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };

    let runs = box_runs(&s);
    assert!(
        runs.iter().any(|r| r == MESSAGE),
        "{FIXTURE} r{RELEASE}: the box must carry {MESSAGE:?} as one painted run — \
         `print_form` (EXT:0x1A) is how it gets there. Painted runs found: {runs:?}"
    );

    let mut checked = 0usize;
    for honor in [true, false] {
        for &(w, h) in PANES {
            let st = state(honor);
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ =
                app::render::screen::render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
            let lines = rows(&buf, area);
            assert!(
                lines.iter().any(|r| r.contains(MESSAGE)),
                "{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: {MESSAGE:?} must be drawn with \
                 glyphs:\n{}",
                lines.join("\n")
            );
            checked += 1;
        }
    }
    assert_eq!(checked, PANES.len() * 2, "every pane measured in both colour modes");
}

/// **SQ-1008.** A pane taller than the game's own screen must not cost the game
/// a window.
///
/// The story viewport's growth into the letterbox slack is deliberate and stays:
/// window 0 is a scrolling buffer, and the reclaimed rows are how the player
/// reads more than its eleven native rows of history. What it may not grow over
/// is the row the game is using — Arthur puts window 3 at native
/// `(28, 384, 584, 16)`, the last text row of his 640x400 screen, and prints the
/// boxed answer to `hint` into it.
///
/// So both halves are asserted together, because either alone is satisfiable by
/// a wrong fix: the viewport is TALLER than its native 11 rows (the reclaim
/// survived — clamping it to the native rect would have passed the box test and
/// cost the transcript its history), and it STOPS ABOVE the box, which is drawn
/// on the pane's last row through the same bottom-anchored scale Journey's
/// command strip uses.
///
/// Non-vacuity guard: the 25-row panes are the contrast, and they must still be
/// on the letterbox plan with an unreclaimed 11-row viewport. If Arthur ever
/// stops taking a reclaiming plan at 34 rows this case would pass by measuring
/// nothing, so the taller panes assert the growth itself.
#[test]
fn the_box_survives_the_reclaim() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, _)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };

    let mut checked = 0usize;
    for honor in [true, false] {
        for &(w, h) in PANES {
            let st = state(honor);
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ =
                app::render::screen::render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
            let lines = rows(&buf, area);
            let box_row = lines.iter().position(|r| r.contains(MESSAGE)).unwrap_or_else(|| {
                panic!("{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: no box row:\n{}", lines.join("\n"))
            }) as u16;
            let vp = viewport_cells(&st);
            let (vp_top, vp_rows) = (vp.1, vp.3);

            assert!(
                vp_top + vp_rows <= box_row,
                "{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: the story viewport (rows \
                 {vp_top}..{}) must stop above window 3's row {box_row} — native {NATIVE_BOX:?}",
                vp_top + vp_rows
            );

            if h > 25 {
                assert_eq!(
                    box_row,
                    h - 1,
                    "{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: the box is the game's last \
                     native row, so a reclaimed pane bottom-anchors it"
                );
                assert!(
                    vp_rows > NATIVE_STORY_ROWS,
                    "{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: the reclaim must survive — a \
                     {vp_rows}-row viewport is no more than window 0's own \
                     {NATIVE_STORY_ROWS} native rows, so the transcript lost its history"
                );
            } else {
                // The contrast the report was found on: no slack, no reclaim.
                assert_eq!(
                    (vp_rows, box_row),
                    (NATIVE_STORY_ROWS, h - 1),
                    "{FIXTURE} r{RELEASE} {w}x{h} honor={honor}: Arthur's own screen exactly — \
                     an unreclaimed viewport and the box on the last row"
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, PANES.len() * 2, "every pane measured in both colour modes");
}

/// The half of the report that was NOT a defect, pinned so nobody "fixes" it:
/// a boxed message is painted into window 3, and window 3 is not a flowing-prose
/// window, so it does not belong in the host transcript. The turn's transcript
/// is empty, and the turn before it — same `@window_size` on window 0 — is not.
#[test]
fn the_box_is_painted_and_stays_out_of_the_transcript() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, transcript)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };
    assert!(
        !transcript.contains("crystal ball"),
        "{FIXTURE} r{RELEASE}: a boxed message is paint, not prose — it must not be spliced \
         into the scrolling transcript. Got {transcript:?}"
    );
    // …and it really is on the screen, so the assertion above is about ROUTING
    // and not about the message having gone missing again.
    assert!(
        box_runs(&s).iter().any(|r| r == MESSAGE),
        "{FIXTURE} r{RELEASE}: the box must still hold the message"
    );
}

/// Arthur reads the formatted table's own line records to decide how tall to
/// make the box, so the table's SHAPE is observable in the game's geometry: a
/// 34-character message justified to window 0's 584 px is one 16-px line. Six
/// lines (96 px) is what the plain §7.1.2.1 layout produced, walked as if it
/// were formatted text.
///
/// This is the case that fails on the unfixed engine even if `print_form` were
/// somehow satisfied another way — the two halves of the fix are independent
/// and both are needed.
#[test]
fn the_box_is_sized_for_one_line() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, _)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };
    let v6 = s.machine.screen.v6.as_ref().expect("{FIXTURE}: a v6 screen");
    let boxw = &v6.windows[3];
    assert_eq!(
        (boxw.y_size, boxw.x_size),
        (16, 584),
        "{FIXTURE} r{RELEASE}: the game sizes its box from the formatted table's line records — \
         one line for a {}-character message in a 584px box",
        MESSAGE.chars().count()
    );
}
