//! SQ-0756: `/dump-windows` has to describe the last frame the GAME drew, and it
//! has to hand that description over in a form a user can actually copy.
//!
//! Both halves come from one real capture. The command is reached through the
//! command palette or a hotkey dialog, and both are modal overlays that route the
//! v6 pane off its pixel path — so the frame the dump read was always the modal's,
//! and every window in the user's paste said `cells: NOT DRAWN this frame`, which
//! is exactly the question the command exists to answer. The same paste was also
//! unreadable: selecting the dump off a v6 pane takes the kitty unicode
//! placeholder glyphs with it, and whole fields came back truncated mid-word.
//!
//! These cases use a synthetic v6 `Layered` model rather than a story file, so
//! they run in CI where `stories/` is absent: what is under test is the frame
//! BOOKKEEPING and the delivery, neither of which needs a real game.

use app::engine::{BufferWindow, GraphicsWindow, PositionedWindow, ScreenModel, StatusModel, WinNode};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// A native 320x200 v6 screen: an opaque full-screen chrome graphics window with
/// the primary story buffer inset in it — enough for the hybrid ring to have a
/// ring to draw and a story viewport to place.
fn v6_model() -> ScreenModel {
    let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
    let chrome = PositionedWindow {
        x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
        left_margin: 0, right_margin: 0,
        node: WinNode::Graphics(GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(chrome_img),
            version: 1,
            upscale: false,
        }),
    };
    let story = PositionedWindow {
        x: 5, y: 4, w: 29, h: 20, x_px: 43, y_px: 39, w_px: 234, h_px: 160,
        left_margin: 0, right_margin: 0,
        node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
    };
    ScreenModel {
        root: WinNode::Layered(vec![chrome, story]),
        status: StatusModel::HostManaged,
        bg: 0,
        fg: 0,
        content_size: (40, 25),
    }
}

fn v6_state() -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.push_transcript("HELLO STORY WORLD");
    state
}

fn draw(state: &AppState, model: &ScreenModel, cols: u16, rows: u16) {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
}

/// The render path a cell map records, `/dump-windows`'s `render path:` line.
fn path_of(cells: &[app::state::V6CellRect]) -> String {
    cells
        .iter()
        .find(|c| c.label.starts_with("path:"))
        .map(|c| c.label.clone())
        .unwrap_or_else(|| "<none>".into())
}

fn pane_of(cells: &[app::state::V6CellRect]) -> (u16, u16, u16, u16) {
    cells.iter().find(|c| c.label == "pane").map(|c| c.cells).unwrap_or_default()
}

/// The rect the frame's own path entry carries — the ring records the pane, the
/// cell path the story box. Every frame has one, whichever path drew it.
fn path_rect(cells: &[app::state::V6CellRect]) -> (u16, u16, u16, u16) {
    cells.iter().find(|c| c.label.starts_with("path:")).map(|c| c.cells).unwrap_or_default()
}

/// (a) The dump describes the GAME's frame while the modal that summoned it is
/// still up — its render path, its pane, its window placements.
///
/// The two frames are deliberately different in every reported way: the game frame
/// takes the hybrid ring at 40x25, the modal frame takes the cell path at 30x20.
/// A dump that reports either frame's numbers therefore cannot pass as the other,
/// which is the whole point — the old behaviour reported the modal's.
///
/// This case drives the SUPPLIER, `AppState::v6_dump_frame`, and reverting it to
/// read `state.v6_cell_map` makes the reported path `cell — modal overlay open:
/// hotkey_dialog` and the pane 30x20. It does NOT reach the dispatch arm that
/// calls it — that guard lives with the arm, in `slash_dispatch.rs`'s
/// `dump_windows_reports_the_game_frame_and_names_its_log`, because a
/// `pub(crate)` binary function is not callable from an integration test
/// (SQ-0777: this comment used to claim a falsification nobody could run here).
#[test]
fn the_dump_describes_the_last_game_frame_not_the_modals() {
    let model = v6_model();
    let state = v6_state();

    // One ordinary game frame.
    draw(&state, &model, 40, 25);
    let game_path = path_of(&state.v6_cell_map.borrow());
    assert_eq!(game_path, "path:hybrid-ring", "the game frame takes the pixel ring");

    // …then the frames the command itself causes: a hotkey dialog opens (the
    // second of the two routes into this command) and the pane redraws under it.
    let mut state = state;
    state.overlays.hotkey_dialog = true;
    draw(&state, &model, 30, 20);
    draw(&state, &model, 30, 20);

    // The live map is the modal's, and says so — the precondition of the defect.
    {
        let live = state.v6_cell_map.borrow();
        assert_eq!(
            path_of(&live),
            "path:cell — modal overlay open: hotkey_dialog",
            "the frame standing in v6_cell_map when the command runs is the modal's"
        );
        assert!(path_rect(&live).2 <= 30, "…drawn at the modal frame's size: {:?}", path_rect(&live));
    }

    // What the dump reports is the game's frame, not that one.
    let (frame, line) = state.v6_dump_frame();
    let frame = frame.expect("a game frame was drawn, so one is on record");
    assert_eq!(path_of(&frame.cells), "path:hybrid-ring", "the reported render path is the game's");
    assert_eq!(pane_of(&frame.cells), (0, 0, 40, 25), "the reported pane is the game frame's, not the modal's");
    assert!(
        frame.cells.iter().any(|c| c.label == "viewport"),
        "the game frame's story viewport is reported: {:?}",
        frame.cells.iter().map(|c| &c.label).collect::<Vec<_>>()
    );
    assert!(
        frame.cells.iter().any(|c| c.native == (43, 39, 234, 160)),
        "the story window's placement is reported, so it can never read NOT DRAWN while a \
         modal is merely open: {:?}",
        frame.cells.iter().map(|c| (&c.label, c.native)).collect::<Vec<_>>()
    );
    assert_eq!(frame.modal_frames_since, 2, "both modal frames are counted, and neither is described");
    assert!(line.contains("the last frame the game drew"), "the dump says which frame it describes: {line}");
    assert!(line.contains("2 modal frame(s) ago"), "…and how stale that is: {line}");
}

/// (b) A frame the game draws after the modal closes replaces the record, and the
/// staleness count goes back to zero — the snapshot follows the game, it is not a
/// one-shot taken at the first frame of the session.
#[test]
fn a_later_game_frame_replaces_the_record() {
    let model = v6_model();
    let mut state = v6_state();

    draw(&state, &model, 40, 25);
    state.overlays.hotkey_dialog = true;
    draw(&state, &model, 30, 20);
    state.overlays.hotkey_dialog = false;
    draw(&state, &model, 52, 30);

    let (frame, line) = state.v6_dump_frame();
    let frame = frame.expect("a game frame is on record");
    assert_eq!(pane_of(&frame.cells), (0, 0, 52, 30), "the newest GAME frame is the one described");
    assert_eq!(frame.modal_frames_since, 0, "nothing modal has been drawn since");
    assert!(line.contains("the frame on screen now"), "no modal frames to disclaim: {line}");
}

/// (c) With no game frame ever drawn, the placements are unknown — and the dump
/// says so rather than quietly reporting the modal's, which is the failure mode
/// this whole quest is about.
#[test]
fn no_game_frame_reports_unavailable_rather_than_the_modals() {
    let model = v6_model();
    let mut state = v6_state();
    state.overlays.hotkey_dialog = true;
    draw(&state, &model, 30, 20);

    let (frame, line) = state.v6_dump_frame();
    assert!(frame.is_none(), "a modal frame is not a game frame, and must not stand in for one");
    assert!(line.contains("UNAVAILABLE"), "the missing fields are named as missing: {line}");
}

/// (d) The dump is delivered to a file, byte for byte.
///
/// The user's on-screen copy came back with fields truncated mid-word
/// (`mp-windows` for `dump-windows`) because a v6 pane clips at its width and the
/// selection dragged the graphics protocol's placeholder glyphs along with it. A
/// line far longer than any pane must survive whole, and the file must hold
/// nothing but the dump.
#[test]
fn the_dump_is_written_to_a_copyable_log_verbatim() {
    let dir = std::env::temp_dir().join(format!("lanthorn-dumpwin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let long = format!("  win1  640x400 at (1,1)  {}", "x".repeat(400));
    let first = vec!["v6 layout — current window 2, input window 2".to_string(), long.clone()];
    let path = app::export::append_window_dump(&dir, &first).expect("the log is written");
    assert_eq!(path, app::export::window_dump_path(&dir));

    let text = std::fs::read_to_string(&path).unwrap();
    for line in &first {
        assert!(
            text.lines().any(|l| l == line),
            "every dump line lands in the log unclipped and unaltered: {line:?} not in {text:?}"
        );
    }
    assert!(
        !text.chars().any(|c| ('\u{0e000}'..='\u{10ffff}').contains(&c)),
        "the log carries no kitty unicode placeholder glyphs — that corruption comes from \
         selecting the drawn pane, and this text never went through it"
    );

    // A second capture appends: an investigation takes several, before and after a
    // move, and each describes a different frame.
    let second = vec!["  ring: plan tall, ring not clipped".to_string()];
    app::export::append_window_dump(&dir, &second).expect("the second capture appends");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(&long), "the first capture is still there");
    assert!(text.contains("ring not clipped"), "and the second is below it");
    assert_eq!(text.matches("=== /dump-windows ").count(), 2, "one stamped header per capture");

    let _ = std::fs::remove_dir_all(&dir);
}
