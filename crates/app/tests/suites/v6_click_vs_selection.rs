//! SQ-1378: in a v6 game a CLICK goes to the game and a DRAG selects text.
//!
//! The player reported that story-pane mouse text selection — press, drag,
//! release, "Copied N chars" — works in text games and does nothing at all in
//! Zork Zero, Shogun and Arthur. The cause is SQ-0566's click delivery: a left
//! press inside the drawn v6 image was handed to the VM on the DOWN, as ZSCII 254
//! (ZMSD §3.8) or as a line-read terminator, and the event never reached
//! `Action::StartSelection`. `V6ClickMap::map_click` covers the story TEXT as well
//! as the artwork, so every press in the pane of a story whose read accepts a
//! click ended that read — which is precisely these three games and not Journey,
//! whose terminating-characters table is empty.
//!
//! The fix defers: a press records the click, a drag drops it and selects text, a
//! release with no drag in between delivers it exactly as the Down used to. The
//! whole decision is `app::input::v6_mouse_outcome`, so these cases drive the
//! production answer rather than a re-implementation of it — reverting the
//! deferral (a press answering `Deliver`) fails them with the read consumed and no
//! selection text, which is the reported symptom.
//!
//! Specimens, with the turn count each frame is reached at (a frame is a fixture):
//!
//! | fixture                    | release | to the prompt                        |
//! |----------------------------|---------|--------------------------------------|
//! | `zork0-r393-s890714.z6`    | 393     | boots straight to a LINE read — 0     |
//! | `shogun-r322-s890706.z6`   | 322     | Enter past the title menu — ≤8 inputs |
//! | `arthur-r74-s890714.z6`    | 74      | tap the intro, `n` to restore — 12    |
//!
//! Skip-if-missing (the stories are gitignored), and non-vacuous: a fixture that
//! is present but whose frame yields no click map, no on-screen marker line or no
//! click terminator fails rather than passing quietly.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::input::{apply_action, mouse_to_action, v6_mouse_outcome, Action, V6MouseOutcome};
use app::session::{GameSession, InputKind};
use app::state::AppState;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use mapper::mapper::Mapper;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The pane every case renders into. Wide enough that the marker line below fits
/// on one row, tall enough for the chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 100, height: 34 };

/// The map pane the mouse router is told about — deliberately empty, so every
/// event routes to the story pane.
const NO_MAP: Rect = Rect { x: 0, y: 0, width: 0, height: 0 };

/// A line the player could drag across: pushed into the transcript, found in the
/// rendered buffer, and expected back out of the clipboard.
const MARKER: &str = "PICK THIS UP";

/// Boot a press and drive it to a LINE read, or `None` (with a SKIP note) when the
/// gitignored story is absent.
fn boot(file: &str, release: u16, intro_taps: usize) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file} is not the pinned release"
    );
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session = GameSession::new_with_trace(
        bytes, true, false, None, false, picture_dims, std_window, None, None,
    )
    .expect("the press should load and boot without a ZError");
    assert!(!session.quit, "{file} quit during boot");
    assert!(session.machine.fault_trace.is_none(), "{file} faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap through whatever intro stands between boot and the `>` prompt,
    // answering the "restore a saved position?" question with 'n'.
    for _ in 0..intro_taps {
        if matches!(session.pending_input(), InputKind::Line) {
            break;
        }
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    let _ = session.take_transcript();
    Some(session)
}

/// An app state wired for a headless hybrid v6 render — the shipped default mode,
/// where the story window really is terminal text and therefore selectable.
fn render_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state
}

/// Draw the frame the player is looking at. This is what publishes the click map
/// (`last_v6_map`), the transcript geometry the selection inverts through, and —
/// while a selection is live — the text it covers.
fn draw(state: &AppState, session: &GameSession) -> Buffer {
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, PANE, &mut buf);
    buf
}

/// Where `needle` was drawn, as (column of its first character, row).
fn find_on_screen(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
    for y in PANE.y..PANE.bottom() {
        let row: String = (PANE.x..PANE.right())
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        if let Some(idx) = row.find(needle) {
            // Columns, not bytes: the chrome ring is drawn in box-drawing glyphs,
            // and each of those is three bytes of this row string.
            return Some((PANE.x + row[..idx].chars().count() as u16, y));
        }
    }
    None
}

fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
    MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }
}

/// What the run loop asks about a press: the game pixel under it, if any.
fn hit(state: &AppState, col: u16, row: u16) -> Option<(u16, u16)> {
    state.graphics_render.borrow().last_v6_map.as_ref().and_then(|cm| cm.map_click(col, row))
}

/// The run loop's own three facts about the pending read.
fn read_facts(session: &GameSession) -> (Option<InputKind>, Option<u8>) {
    (Some(session.pending_input()), session.mouse_click_terminator())
}

/// Press, drag two cells along a line of story text, release — and assert the
/// player gets a text selection and the VM is never told anything.
fn drag_selects_text_rather_than_clicking(file: &str, release: u16, intro_taps: usize) {
    let Some(session) = boot(file, release, intro_taps) else { return };

    // The gate: this story is one of the three where a press used to be eaten.
    assert!(
        matches!(session.pending_input(), InputKind::Line),
        "{file}: expected the `>` prompt, got {:?}",
        session.pending_input()
    );
    assert_eq!(
        session.mouse_click_terminator(),
        Some(254),
        "{file}: this suite is only about stories whose line read accepts a click"
    );

    let mut state = render_state();
    let mut mapper = Mapper::default();
    state.push_transcript(MARKER);
    state.push_transcript("You are standing in an open field west of a white house.");

    let buf = draw(&state, &session);
    let (col, row) = find_on_screen(&buf, MARKER)
        .unwrap_or_else(|| panic!("{file}: the marker line must be drawn in the story viewport"));

    // The reproduction condition: this cell is inside the game's own screen, so
    // before SQ-1378 the press below was delivered to the VM as a click and the
    // selection never started.
    let press_px = hit(&state, col, row)
        .unwrap_or_else(|| panic!("{file}: a cell of story text maps into the game screen"));

    // ── Down: the click is remembered, and the event still anchors a selection ──
    let (read, term) = read_facts(&session);
    let down = mouse(MouseEventKind::Down(MouseButton::Left), col, row);
    let outcome = v6_mouse_outcome(state.pending_v6_click, read, term, Some(press_px), &down);
    match outcome {
        V6MouseOutcome::DeferAndRoute(click) => {
            assert_eq!(click.game_px, press_px, "{file}: the deferred click is the pixel pressed");
            state.pending_v6_click = Some(click);
        }
        other => panic!("{file}: a press must defer, not {other:?} — this is the reported defect"),
    }
    let action = mouse_to_action(&state, down, NO_MAP, PANE, &[], &None);
    assert!(
        matches!(action, Action::StartSelection(c, r) if (c, r) == (col, row)),
        "{file}: the press still reaches the story pane, got {action:?}"
    );
    apply_action(action, &mut state, &mut mapper);
    assert!(state.selection.is_some(), "{file}: the press anchored a selection");

    // ── Drag across the marker: the click dies, the selection grows ────────────
    let last = col + MARKER.chars().count() as u16 - 1;
    let drag = mouse(MouseEventKind::Drag(MouseButton::Left), last, row);
    let (read, term) = read_facts(&session);
    assert_eq!(
        v6_mouse_outcome(state.pending_v6_click, read, term, None, &drag),
        V6MouseOutcome::ForgetAndRoute,
        "{file}: a drag makes the gesture a selection"
    );
    state.pending_v6_click = None;
    let action = mouse_to_action(&state, drag, NO_MAP, PANE, &[], &None);
    assert!(
        matches!(action, Action::ExtendSelection(c, r) if (c, r) == (last, row)),
        "{file}: the drag extends the selection, got {action:?}"
    );
    apply_action(action, &mut state, &mut mapper);

    // The next frame publishes the text under the selection.
    let _ = draw(&state, &session);

    // ── Up: nothing to deliver, so the release copies ─────────────────────────
    let up = mouse(MouseEventKind::Up(MouseButton::Left), last, row);
    let (read, term) = read_facts(&session);
    assert_eq!(
        v6_mouse_outcome(state.pending_v6_click, read, term, None, &up),
        V6MouseOutcome::Route,
        "{file}: the release has no click left to deliver"
    );
    let action = mouse_to_action(&state, up, NO_MAP, PANE, &[], &None);
    assert!(matches!(action, Action::EndSelection), "{file}: the release ends the selection, got {action:?}");
    let copied = app::input::finish_selection(&mut state)
        .unwrap_or_else(|| panic!("{file}: the drag must copy the text it covered"));
    assert!(
        copied.contains(MARKER),
        "{file}: the clipboard carries the dragged line, got {copied:?}"
    );
    assert!(
        state.transcript.iter().any(|l| l.starts_with("Copied ") && l.contains("chars to clipboard")),
        "{file}: the copy is reported in the story output: {:?}",
        state.transcript
    );
    let n: usize = copied.chars().count();
    assert!(n >= MARKER.chars().count(), "{file}: copied {n} chars");

    // And the VM never heard about any of it: the same read is still pending, and
    // the story printed nothing.
    assert!(
        matches!(session.pending_input(), InputKind::Line),
        "{file}: the line read must survive a text selection"
    );
    assert_eq!(state.turns, 0, "{file}: a selection is not a turn");
}

#[test]
fn zork0_a_drag_over_story_text_selects_instead_of_clicking() {
    drag_selects_text_rather_than_clicking("zork0-r393-s890714.z6", 393, 4);
}

#[test]
fn shogun_a_drag_over_story_text_selects_instead_of_clicking() {
    drag_selects_text_rather_than_clicking("shogun-r322-s890706.z6", 322, 8);
}

#[test]
fn arthur_a_drag_over_story_text_selects_instead_of_clicking() {
    drag_selects_text_rather_than_clicking("arthur-r74-s890714.z6", 74, 12);
}

/// SQ-0566, unchanged and pinned: a press and release with no drag in between on
/// Zork Zero's border compass still ends the line read with terminator 254 and the
/// clicked coordinates — the game synthesizes "north" and the player walks. That
/// is the behaviour the deferral had to keep while making selection possible.
#[test]
fn zork0_a_press_and_release_on_the_compass_still_moves_the_player() {
    let Some(mut session) = boot("zork0-r393-s890714.z6", 393, 4) else { return };
    assert!(session.wants_mouse(), "Zork Zero sets Flags2 bit 5");

    let mut state = render_state();
    let mut mapper = Mapper::default();
    state.push_transcript(MARKER);
    let _ = draw(&state, &session);

    // The rose's north spoke is native (282..372, 6..80) of the 640x400 screen;
    // find the pane cell whose centre lands nearest its middle (322, 12) rather
    // than assuming a scale (SQ-0901: a measured frame beats an assumed one).
    let mut best: Option<(u16, u16, u32)> = None;
    for row in PANE.y..PANE.bottom() {
        for col in PANE.x..PANE.right() {
            if let Some((gx, gy)) = hit(&state, col, row) {
                let d = (gx as i32 - 322).pow(2) as u32 + (gy as i32 - 12).pow(2) as u32;
                if best.is_none_or(|(_, _, bd)| d < bd) {
                    best = Some((col, row, d));
                }
            }
        }
    }
    let (col, row, _) = best.expect("the frame published a click map");
    let (gx, gy) = hit(&state, col, row).expect("the chosen cell maps");
    assert!(
        (282..=372).contains(&gx) && (6..=80).contains(&gy),
        "cell ({col},{row}) → ({gx},{gy}) is outside the compass rose"
    );

    // Down, then Up with no motion in between.
    let (read, term) = read_facts(&session);
    let down = mouse(MouseEventKind::Down(MouseButton::Left), col, row);
    let V6MouseOutcome::DeferAndRoute(click) =
        v6_mouse_outcome(state.pending_v6_click, read, term, Some((gx, gy)), &down)
    else {
        panic!("a press over the compass defers a click");
    };
    state.pending_v6_click = Some(click);
    apply_action(mouse_to_action(&state, down, NO_MAP, PANE, &[], &None), &mut state, &mut mapper);

    let up = mouse(MouseEventKind::Up(MouseButton::Left), col, row);
    let (read, term) = read_facts(&session);
    let V6MouseOutcome::Deliver(click) = v6_mouse_outcome(state.pending_v6_click, read, term, None, &up)
    else {
        panic!("an unmoved release delivers the click");
    };
    state.pending_v6_click = None;
    // The run loop drops the zero-length anchor rather than copying it.
    app::input::discard_selection(&mut state);
    assert!(state.selection.is_none(), "no selection survives a delivered click");
    assert!(
        !state.transcript.iter().any(|l| l.starts_with("Copied ")),
        "a click must not report a copy: {:?}",
        state.transcript
    );

    // …and the delivery itself, exactly as the run loop makes it.
    let app::state::V6ClickRead::Line { terminator } = click.read else {
        panic!("Zork Zero sits at a LINE read");
    };
    assert_eq!(terminator, 254, "ZSCII single-click (ZMSD §3.8)");
    session.set_mouse(click.game_px.1, click.game_px.0); // engine stores (y, x)
    let result = session.submit_line_with_terminator("", terminator);
    assert!(result.fault.is_none(), "faulted on a compass click: {:?}", result.fault);
    assert!(!result.quit, "quit on a compass click");
    assert_eq!(
        app::session::echoed_direction_command(&result.transcript),
        Some("north"),
        "the game synthesized the clicked direction: {:?}",
        result.transcript
    );
}
