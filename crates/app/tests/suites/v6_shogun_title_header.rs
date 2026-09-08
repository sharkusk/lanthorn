//! Shogun's centred title header survives the split that moves the story window
//! out from under it — SQ-0745.
//!
//! The report, Shogun booted from its Amiga release floppy: *"amiga shogun doesn't
//! show the centered text as the menu is rendered at the very top of the page,
//! pushing the centered text off the top of the screen. you can briefly see it if
//! you scroll up (but it snaps back down)."*
//!
//! TWO RELEASES, one of which was already right. The `.adf` is r295/890321 and the
//! bare story file is r322/890706, and they print the same nine centred lines
//! through window 0 and then move window 0 to a 64px strip at the foot of the
//! screen — but by different opcodes:
//!
//! | release | moves window 0 with | header frozen |
//! |---|---|---|
//! | r322/890706 (`shogun-r322-s890706.z6`) | `move_window` + `window_size` | all nine lines |
//! | r295/890321 (`James Clavell's Shogun.adf`) | `@split_window(336)` | one line |
//!
//! SQ-0697 retires prose a window's new box no longer covers, and it hung off
//! `move_window`/`window_size` only. `split_window` places window 0 just as surely
//! (ZMSD §8.8.4.1's tiling, which is SQ-0712's own finding), so on the floppy eight
//! of the nine header lines were still *live* prose in a window the game erased on
//! the very next instruction. They survived only as transcript lines above the
//! screen-clear boundary the erase marks — which is exactly "you can briefly see it
//! if you scroll up, but it snaps back down".
//!
//! Asserted on BOTH releases: r322 is the control that was already correct, and a
//! fix that moves it has broken something.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TurnResult};
use app::state::TranscriptKind;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The Amiga release, still on its release floppy — r295/890321, and the build the
/// report is about. `InterpreterProfile::resolve` reads the medium, so this is a
/// different BUILD of the game and not the bare story file under another palette
/// (CLAUDE.md, "a disk image is a different release").
const AMIGA_RELEASE: &str = "James Clavell's Shogun.adf";

/// The IBM PC release as an ordinary story file — r322/890706. The control.
const PC_RELEASE: &str = "shogun-r322-s890706.z6";

/// The centred header, in the order Shogun prints it. Every line is common to both
/// releases; the release/serial line differs between them and is left out.
const HEADER: [&str; 7] = [
    "SHOGUN",
    "A Story of Japan",
    "Copyright (c) 1988 by Infocom",
    "All rights reserved.",
    "SHOGUN is a trademark of James Clavell",
    "Original Literary Work Copyright 1975 by James Clavell",
    "Licensed by Noble House Trading Limited, London.",
];

/// The prompt the game prints into the re-placed window 0, beside its boot menu.
const PROMPT: &str = "You may choose to:";

/// A boot-menu item, painted through window 2 at its own native rows.
const MENU_ITEM: &str = "RESTORE a saved game";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot the release in `file` the way `startup` does: the story out of whatever
/// container it came in (a disk image holds it as `Story.data`), its own artwork
/// resolved from that same container, under `profile`.
fn boot_release(file: &str, profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join(file);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::resolve(&story_path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Shogun (v6) should load and boot without a ZError");
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    session.machine.set_palette(profile.palette());
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// What `turn::apply_game_driven_result` does with one turn's output.
fn apply(state: &mut app::state::AppState, r: &TurnResult) {
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
    }
}

#[allow(deprecated)]
/// The state these cases render through.
///
/// FRAMELESS, since SQ-0886. What is being asserted is that the header SURVIVES the
/// split — a property of the screen model, read off a pane — and it needs a path
/// that draws the game's text as terminal rows to read it off. Hybrid was that path
/// for this frame until SQ-0886 found that the same routing threw away Shogun's side
/// panels and its ground, and moved the frame to the composite; the composite's own
/// arm on this header is `v6_prose_freeze::shogun_frozen_header_stays_centred_in_
/// every_render_path`, which measures the same nine lines as ink.
fn fresh_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    // Force the CELL path: SQ-0895 removed frameless, which was the
    // deliberate route in. Dropping the picker is the substitute whose
    // ONLY effect is the one frameless contributed here — draw no game
    // image. (A modal overlay also lands on the cell path, but it
    // additionally suppresses the inlined input line, which shifts row
    // counts.)
    state.game_picker = None;
    state.config.honor_game_colours = honor;
    state
}

fn pane_rows(state: &app::state::AppState, model: &app::engine::ScreenModel, pane: (u16, u16)) -> Vec<String> {
    let area = Rect::new(1, 1, pane.0, pane.1);
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// The reported frame: the boot menu, on both releases, at the user's pane and a
/// second size, in both colour modes.
///
/// PERTURBED BEFORE ASSERTING (CLAUDE.md): the header is printed on the boot turn
/// and the split that strands it happens on the FIRST keypress, so the frame the
/// report is about is never the one the game boots into. Every keypress up to the
/// menu is rendered and checked, and the assertion only bites once the menu is up —
/// which is one whole turn past the boundary that breaks it.
///
/// FALSIFICATION — with the split's retire dropped (`split_window`'s v6 arm back to
/// assigning the tiled boxes without freezing what they leave behind):
///
/// > `James Clavell's Shogun.adf Amiga 159x61 honor=true: the centred title header is
/// > gone from the pane — "A Story of Japan" is not on any row, though the boot menu
/// > is up at row 2. The game printed it, then moved window 0 out from under it.`
#[test]
fn shogun_shows_its_centred_title_above_the_boot_menu_on_both_releases() {
    for (file, profile) in
        [(AMIGA_RELEASE, InterpreterProfile::Amiga), (PC_RELEASE, InterpreterProfile::IbmPc)]
    {
        for pane in [(159u16, 61u16), (138, 68)] {
            for honor in [true, false] {
                let Some(mut session) = boot_release(file, profile) else { return };
                let mut state = fresh_state(honor);
                let opening = session.take_transcript();
                state.push_transcript_runs(&opening, TranscriptKind::Story, &[]);
                let ctx = format!("{file} {profile:?} {}x{} honor={honor}", pane.0, pane.1);
                let mut seen_menu = false;
                for _ in 1..=4 {
                    let r = match session.pending_input() {
                        InputKind::Line => session.submit(""),
                        InputKind::Char => session.submit_char(13),
                        InputKind::Event => session.submit(""),
                    };
                    apply(&mut state, &r);
                    let model = session.screen();
                    let rows = pane_rows(&state, &model, pane);
                    let Some(prompt_row) = rows.iter().position(|r| r.contains(PROMPT)) else {
                        continue;
                    };
                    seen_menu = true;

                    // Every header line is ON the pane. The report is that they are
                    // not: they were pushed above the viewport, reachable only by a
                    // scroll that the bottom-stick immediately undoes.
                    let mut at = Vec::new();
                    for line in HEADER {
                        let row = rows.iter().position(|r| r.contains(line)).unwrap_or_else(|| {
                            panic!(
                                "{ctx}: the centred title header is gone from the pane — {line:?} is \
                                 not on any row, though the boot menu is up at row {prompt_row}. The \
                                 game printed it, then moved window 0 out from under it.\n{}",
                                rows.join("\n")
                            )
                        });
                        at.push(row);
                    }

                    // …in the order the game printed them, one per row.
                    assert!(
                        at.windows(2).all(|w| w[0] < w[1]),
                        "{ctx}: the header reads top-down in the order it was printed — got rows \
                         {at:?} for {HEADER:?}\n{}",
                        rows.join("\n")
                    );

                    // …and the menu is BELOW all of it, not over the top of it, which
                    // is how the defect first read.
                    let last = *at.last().expect("HEADER is not empty");
                    assert!(
                        prompt_row > last,
                        "{ctx}: the boot menu is drawn at row {prompt_row}, over the header that ends \
                         at row {last}\n{}",
                        rows.join("\n")
                    );
                    let menu_row = rows.iter().position(|r| r.contains(MENU_ITEM)).unwrap_or_else(|| {
                        panic!("{ctx}: the boot menu's items are on the pane\n{}", rows.join("\n"))
                    });
                    assert!(
                        menu_row > last,
                        "{ctx}: the menu item {MENU_ITEM:?} is drawn at row {menu_row}, over the header \
                         that ends at row {last}\n{}",
                        rows.join("\n")
                    );
                    break;
                }
                assert!(seen_menu, "{ctx}: the boot menu is reached within four keypresses");
            }
        }
    }
}

/// The engine half, and the one that says WHY: the split hands the whole header over
/// to paint, on both releases.
///
/// `ZWindow::retired` is prose the window's new box no longer covers, frozen where it
/// was drawn; whatever the split leaves in `streamed` instead is still the host
/// transcript's own live screen (the same routing decision the print path makes, per
/// SQ-0755's `v6_holds_host_prose`) — and Shogun erases window 0 on the very next
/// instruction, so on the floppy eight of the nine header lines were destroyed there.
///
/// Falsified with the split's retire dropped:
///
/// > `James Clavell's Shogun.adf: "A Story of Japan" was handed over to paint —
/// > retired runs are ["SHOGUN"]`
///
/// One line frozen out of nine: the `move_window` that precedes the split shrinks
/// window 0 to a box that still covers eight of them.
#[test]
fn the_split_hands_shoguns_header_over_to_paint() {
    for (file, profile) in
        [(AMIGA_RELEASE, InterpreterProfile::Amiga), (PC_RELEASE, InterpreterProfile::IbmPc)]
    {
        let Some(mut session) = boot_release(file, profile) else { return };
        // One keypress: the header is printed on the boot turn, and the window move
        // that strands it happens here.
        match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
        let v6 = session.machine.screen.v6.as_ref().expect("Shogun is a v6 story");
        let painted: Vec<&str> = v6.windows[0].retired.iter().map(|t| t.text.as_str()).collect();
        for line in HEADER {
            assert!(
                painted.iter().any(|t| line.contains(t.trim()) && !t.trim().is_empty()),
                "{file}: {line:?} was handed over to paint — retired runs are {painted:?}"
            );
        }
    }
}
