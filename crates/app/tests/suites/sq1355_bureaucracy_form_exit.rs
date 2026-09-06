//! SQ-1355: *Bureaucracy*'s licence form hands back to a screen the player can
//! read, without buying it with a wasted turn.
//!
//! # What the game does
//!
//! `DRAW-FORM` (`forms.zil`) opens the form with `<CLEAR -1>` and `<SPLIT <-
//! ,HEIGHT 1>>` — an upper window one row short of the whole screen — and reads
//! every keystroke with `<INPUT 1>`, so in lanthorn's model each character is a
//! turn. `FILL-FORM` closes it with
//!
//! ```text
//!   <CLEAR ,S-WINDOW>      ; erase_window 1 — the whole 24-row upper window
//!   <CLEAR ,S-TEXT>        ; erase_window 0
//!   <INIT-STATUS-LINE>     ; <SPLIT 1> <SCREEN 1> …status… <SCREEN 0>
//! ```
//!
//! and returns into `GO` (`other-misc.zil`), which prints the intro, the
//! BUREAUCRACY banner (`V-VERSION`) and the first room (`V-LOOK`) before the
//! main loop's first `read`. All of that is ONE turn — the turn the last Enter
//! of the form starts.
//!
//! # What went wrong
//!
//! Nothing in the transcript: that turn always carried the banner and *Front
//! Room*. The screen did. `split_window` keeps the rows a shrink leaves behind
//! (SQ-0696, the Inform quote box), and it kept them by ALLOCATION rather than
//! by paint — so `<SPLIT 1>` over a 24-row upper window the game had just ERASED
//! preserved 24 blank rows. The host renders the upper grid at its full height,
//! which left the story pane with no rows at all: a blank screen until the
//! player pressed Enter, whose `supply_line` ran `retire_stranded_upper_rows`.
//! That Enter is an empty command, and *Bureaucracy* charges for it —
//! `[What?]` and `[Your blood pressure just went up.]`.
//!
//! `machine-screenshots/dos-bureaucracy.png` is the oracle: on Infocom's own
//! IBM interpreter the banner and *Front Room* are on screen the moment the form
//! ends, under a one-row `Front Room … Blood Pressure:` status line.
//!
//! # The specimen
//!
//! | fixture | release / serial | turns in |
//! |---|---|---|
//! | `stories/bureaucracy-r116-s870602.z4` | 116 / 870602 | 1 line ("start"), 1 key, then 14 form fields |
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously without
//! it.

use std::path::PathBuf;

use app::colors::ColorScheme;
use app::render::screen::render_story_pane;
use app::session::{screen_model_from_machine, GameSession, InputKind, TurnResult};
use app::state::{AppState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const STORY: &str = "bureaucracy-r116-s870602.z4";
/// The screen the game is told about, and the pane the frames below are drawn
/// into. `HEIGHT` drives `DRAW-FORM`'s `<SPLIT <- ,HEIGHT 1>>`, so this is the
/// 24 rows the defect strands.
const SCREEN: (u16, u16) = (25, 80);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the story on a 25x80 screen, as `startup.rs` does for a plain story file
/// on no medium: no archive, no picture dims, the host pane seeded before boot
/// (SQ-0680) so `GO`'s `<LOWCORE SCRV>`/`<LOWCORE SCRH>` read the real screen.
fn boot() -> Option<GameSession> {
    let path = stories_dir().join(STORY);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some(SCREEN),
    )
    .expect("Bureaucracy r116 should load and boot without a ZError");
    assert_eq!(s.machine.mem.version(), 4, "Bureaucracy is a Version 4 story");
    assert_eq!(
        u16::from_be_bytes([s.machine.mem.read_byte(2), s.machine.mem.read_byte(3)]),
        116,
        "this suite is pinned to release 116"
    );
    Some(s)
}

/// The upper window as text, one line per row.
fn upper(s: &GameSession) -> String {
    let up = &s.machine.screen.upper;
    (0..up.rows as usize)
        .map(|r| {
            let row: String =
                (0..up.cols as usize).map(|c| up.cells[r * up.cols as usize + c].ch).collect();
            format!("{}\n", row.trim_end())
        })
        .collect()
}

/// The story pane as the player sees it: the upper grid over the transcript,
/// drawn by the shipped composite (`render::screen::render_story_pane`, the
/// `is_simple` Z-machine arm) into a `SCREEN`-sized pane.
fn pane(s: &GameSession, state: &AppState) -> Vec<String> {
    let area = Rect::new(0, 0, SCREEN.1, SCREEN.0);
    let mut buf = Buffer::empty(area);
    let model = screen_model_from_machine(&s.machine);
    let _ = render_story_pane(&model, false, None, state, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// Fold one game-driven (`read_char`) turn into the app state, exactly as
/// `turn::apply_game_driven_result` does for a keypress: an `erase_lower`
/// collapses the previous reprint back to the clear anchor and re-anchors
/// (SQ-0407), and the turn's output is pushed under that anchor.
fn apply_keypress_turn(state: &mut AppState, r: &TurnResult) {
    assert!(
        r.transcript_elems.is_empty(),
        "this turn takes the flat transcript path; an interleave would need \
         `state::apply_transcript_elems` instead"
    );
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
}

/// Boot, answer `DO-FORM?`'s RESTORE question, press the "[Press any key to
/// begin.]" key, and fill all fourteen licence-form fields.
///
/// The field ORDER is `PICK-FIELD`'s `ZRANDOM`, and each field has its own
/// validator (`FF-SEX` takes only M/F, `FF-STREET-NUMBER` only digits, …), so
/// the driver does not script values: it offers a character, reads the form's
/// own `ERROR:` line out of the upper window, and tries the next candidate.
///
/// Returns the `TurnResult` of the turn that COMPLETED the form — the one whose
/// screen this suite is about.
fn fill_the_form(s: &mut GameSession) -> TurnResult {
    assert_eq!(s.pending_input(), InputKind::Line, "GO's DO-FORM? reads a line");
    s.submit("start");
    assert_eq!(s.pending_input(), InputKind::Char, "…then [Press any key to begin.]");
    s.submit_char(13);
    assert_eq!(s.pending_input(), InputKind::Char, "FILL-FIELD reads the form char by char");

    // Non-vacuity: the form really is on screen, in the tall upper window the
    // defect strands. Without this a driver that silently fell out of the form
    // would assert about a screen the game never drew.
    let form = upper(s);
    assert!(
        form.contains("SOFTWARE LICENCE APPLICATION") && form.contains("Last name:"),
        "the licence form must be drawn in the upper window before it can be \
         filled in; upper was:\n{form}"
    );
    assert_eq!(
        s.machine.screen.upper_window_rows,
        SCREEN.0 - 1,
        "DRAW-FORM splits off HEIGHT-1 rows for the form"
    );

    let mut last = None;
    for _ in 0..30 {
        if s.pending_input() != InputKind::Char {
            break;
        }
        for &cand in b"5MA" {
            s.submit_char(cand);
            if !upper(s).contains("ERROR:") {
                break;
            }
        }
        last = Some(s.submit_char(13));
    }
    let last = last.expect("the form asked for at least one field");
    assert_eq!(
        s.pending_input(),
        InputKind::Line,
        "the form should end at the game's own read prompt; upper was:\n{}",
        upper(s)
    );
    last
}

/// The whole quest, on the frame the player is actually handed.
///
/// Falsified by restoring `split_window`'s allocation-based preserve in
/// `crates/zvm/src/cpu/exec.rs` (`let painted = self.screen.upper.rows.max(rows);`):
///
/// ```text
/// the banner and the first room must be on screen the moment the form ends —
/// "A Paranoid Fantasy" is not in the pane. Pane was:
///  Front Room                                             Blood Pressure: 125/82
///
///
/// …23 blank rows…
/// ```
///
/// which is the reported screen exactly: the status line the game just painted,
/// and nothing under it.
#[test]
fn the_turn_that_completes_the_form_hands_back_a_readable_screen() {
    let Some(mut s) = boot() else { return };
    let r = fill_the_form(&mut s);

    // The host half first, because it is the thing the player sees: the pane
    // they are looking at the instant the form ends.
    let mut state = AppState::default();
    state.colors = ColorScheme::terminal_default();
    apply_keypress_turn(&mut state, &r);
    let pane = pane(&s, &state);
    let flat = pane.join("\n");

    for want in ["A Paranoid Fantasy", "Front Room"] {
        assert!(
            flat.contains(want),
            "the banner and the first room must be on screen the moment the form \
             ends — {want:?} is not in the pane. Pane was:\n{flat}"
        );
    }
    // …and the status line the game just painted is still there, one row of it.
    // The room name has to be found BELOW that row: `INIT-STATUS-LINE` prints
    // "Front Room" into the status bar too, so a pane-wide search for it is
    // answered by the chrome and says nothing about the story.
    assert!(
        pane[0].contains("Blood Pressure"),
        "row 0 is INIT-STATUS-LINE's status bar; pane row 0 was {:?}",
        pane[0]
    );
    assert!(
        pane[1..].iter().any(|l| l.contains("Front Room")),
        "and the room name is in the STORY pane below it, not only in the status \
         bar. Pane was:\n{flat}"
    );

    // The engine half, stated so a failure names the cause as well as the
    // symptom: the split is one row and so is the grid. Twenty-three of those
    // rows were erased by `<CLEAR ,S-WINDOW>` before `<SPLIT 1>` ran, so there is
    // nothing left down there for the host to reserve.
    assert_eq!(s.machine.screen.upper_window_rows, 1, "INIT-STATUS-LINE splits one row off");
    assert_eq!(
        s.machine.screen.upper.rows, 1,
        "and the grid follows it: the rows below the split hold nothing (SQ-1355)"
    );
}

/// The cost the defect charged, stated as a fact about this game: the extra
/// Enter is an empty command, and *Bureaucracy* answers it with a blood-pressure
/// penalty. Nothing the player has to do to see the screen may cost a turn.
#[test]
fn no_extra_keypress_is_needed_to_see_the_screen() {
    let Some(mut s) = boot() else { return };
    let r = fill_the_form(&mut s);

    let mut state = AppState::default();
    state.colors = ColorScheme::terminal_default();
    apply_keypress_turn(&mut state, &r);
    let pane = pane(&s, &state);
    assert!(
        pane[1..].iter().any(|l| l.contains("Front Room")),
        "the room is readable below the status bar before any further input. \
         Pane was:\n{}",
        pane.join("\n")
    );

    // What that extra Enter used to buy, and what it costs.
    let penalty = s.submit("").transcript;
    assert!(
        penalty.contains("[What?]") && penalty.contains("blood pressure"),
        "an empty command is a charged turn in this game, so it must never be the \
         price of a visible screen; it answered: {penalty:?}"
    );
}
