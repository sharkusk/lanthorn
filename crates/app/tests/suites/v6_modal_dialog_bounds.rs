//! SQ-1092 — a modal over a Version 6 game centres in the WHOLE pane, because
//! the frame it would otherwise have to avoid is not drawn while it is open.
//!
//! The reported symptom, on `stories/zork0-r393-s890714.z6` (release 393 /
//! serial 890714) at an 82x34 terminal: the leader panel (`ctrl+p`) lands low
//! and right, its top border around row 26 of 34, with its bottom edge and its
//! `Done` button off the pane entirely. The command palette (`ctrl+p` then `/`)
//! places identically, because every common dialog centres in the one rect
//! `overlays::draw_all` computes:
//!
//! ```ignore
//! let story_bbox = screen::content_bounds(screen_model, story_area);
//! let dialog_area = screen::dialog_bounds(screen_model, story_bbox, full, &state.colors);
//! ```
//!
//! `dialog_bounds` subtracts every graphics window in the tree, because a
//! terminal image protocol draws OVER cells and a dialog centred under one is
//! invisible however late it was written (SQ-0203, Glulx). A v6 story's tree is
//! a `Layered` composite whose graphics leaves carry the game's NATIVE cell rect
//! — Zork Zero's border window is (0, 0, 80, 25) on a 640x400 screen at an 8x16
//! cell — so the subtraction cut an 82x34 frame down to the nine rows below it
//! and every dialog was centred in THAT.
//!
//! Two things are wrong with it at once, and this suite pins both:
//!
//! 1. The rect is not where any dialog should go. It is a bottom strip, so a
//!    dialog placed in it sits low.
//! 2. The rect is SMALLER THAN THE DIALOG. `dialog::centered_rect` clamps to its
//!    bounds, so a panel wanting fifteen rows got nine and lost its buttons.
//!
//! And that exclusion bought nothing, because **a modal forces the v6 pixel path
//! off**: `render_node`'s `Layered` arm is gated on
//! `!state.any_modal_overlay_open()`, and every consumer of `dialog_area` is one
//! of the modals that gate names. The frame the dialog would be avoiding is not
//! on screen to avoid. Dropping to text-only for the dialog is precisely what
//! frees the pane; the placement has to follow it.
//!
//! **But not to nothing.** The cell path draws no frame art with ONE deliberate
//! exception — a chrome graphics window entirely BESIDE the story ("story content,
//! not frame art"), which it still places through the image protocol. Journey's
//! Amiga floppy is that frame, and it is the third case here: its dialog must stay
//! clear of the illustration, and must still get the pane's full height. So the
//! `Layered` arm FILTERS to side columns rather than returning nothing.
//!
//! Fixtures are named by exact release per CLAUDE.md, and `stories/` is
//! gitignored — the Zork Zero case skips vacuously when its fixture is absent.
//! The non-v6 comparison rides on the tracked Mini-Zork I fixture, so it runs on
//! CI too, and it is what says this is a v6-specific placement defect rather
//! than a general one.

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::screen::{content_bounds, dialog_bounds};
use app::machine_boot::MachineBoot;
use app::session::{GameSession, InputKind};

use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// The reported terminal: 82x34 cells.
const FULL: Rect = Rect { x: 0, y: 0, width: 82, height: 34 };

/// The story pane's inner content rect inside that frame — one cell of panel
/// border on every side. This is the `story_area` `draw_frame` hands the ladder;
/// what matters for the defect is only that it is INSET from `full` and that the
/// v6 composite lays its windows out inside it.
const STORY: Rect = Rect { x: 1, y: 1, width: 80, height: 31 };

/// A representative common dialog's requested size: the leader panel is about
/// this shape at this terminal, and every `Placement::Centered` modal resolves
/// through the same `dialog::centered_rect`.
const DIALOG_W: u16 = 60;
const DIALOG_H: u16 = 15;

/// Boot a story the way `startup.rs` does — profile from the medium the mount
/// returned, then the whole `MachineBoot` fact set so no per-machine fact can be
/// omitted (SQ-1021/SQ-1022). Skips (returns `None`) when the fixture is absent.
fn boot(file: &str, release: Option<(u16, &str)>) -> Option<(GameSession, MachineBoot)> {
    let path = fixture_path(file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    if let Some((rel, serial)) = release {
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]),
            rel,
            "{file}: this medium carries a DIFFERENT build than the table says"
        );
        assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    }
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let machine = app::machine_boot::MachineBoot::resolve(
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
    eprintln!(
        "{file}: profile {:?}, release {}, screen_px {:?}, art_scale {:?}, cell {:?}",
        profile,
        u16::from_be_bytes([bytes[2], bytes[3]]),
        machine.screen_px,
        machine.art_scale,
        machine.cell,
    );
    let mut s = GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &machine)
        .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    assert!(!s.quit, "{file}: quit during boot");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some((s, machine))
}

/// The reproduction's own route to the frame: a blank line, then `look`. Four
/// keypresses in the report; the frame this measures is the one two commands in,
/// with the border window up and the story window under it.
fn drive_to_gameplay(s: &mut GameSession) {
    for cmd in ["", "look"] {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit(cmd);
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        if s.quit {
            return;
        }
    }
}

/// The state the ladder reads, built the way `startup.rs` builds it: the theme, and
/// the machine's own `v6_text` (`state.v6_text = boot.text_face()`, `startup.rs:1108`).
/// That face is where BOTH the cell `classify_windows` splits on and the unit screen
/// `native_extent` measures come from, so a harness that let it default would be
/// measuring a screen the player never sees — SQ-1020/SQ-1021 in one line.
fn ladder_state(machine: &MachineBoot) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.v6_text = machine.text_face();
    state
}

/// The two calls `overlays::draw_all` makes, in its order, for the frame this
/// harness booted, at a pane `pane_w` columns wide (the story pane inset one cell
/// on every side, as `draw_frame` hands it over). Returns the frame and the dialog
/// area within it.
fn ladder_dialog_area(session: &GameSession, machine: &MachineBoot, pane_w: u16) -> (Rect, Rect) {
    let model = session.screen();
    let state = ladder_state(machine);
    let full = Rect::new(0, 0, pane_w, FULL.height);
    let story = Rect::new(1, 1, pane_w - 2, FULL.height - 3);
    let bbox = content_bounds(&model, story);
    (full, dialog_bounds(&model, bbox, full, &state))
}

/// Zork Zero, two commands in, at the reported 82x34: the dialog area is the
/// whole frame, so a centred modal is centred and whole.
#[test]
fn zork0_modal_dialog_area_is_the_whole_pane() {
    let Some((mut s, machine)) = boot("zork0-r393-s890714.z6", Some((393, "890714"))) else {
        return;
    };
    drive_to_gameplay(&mut s);

    // NON-VACUITY: this frame must actually be the shape the defect needs — a v6
    // `Layered` composite carrying a graphics window that spans the story. Without
    // this the case passes on any frame with no art at all and proves nothing.
    let model = s.screen();
    let spanning = match &model.root {
        WinNode::Layered(items) => items
            .iter()
            .filter(|pw| matches!(&pw.node, WinNode::Graphics(_)))
            .map(|pw| Rect::new(pw.x, pw.y, pw.w, pw.h))
            .max_by_key(|r| r.width as u32 * r.height as u32),
        other => panic!("Zork Zero must present a v6 Layered root, got {other:?}"),
    };
    let spanning = spanning.expect("this frame carries at least one v6 graphics window");
    assert!(
        spanning.width * spanning.height > (FULL.width as u32 * FULL.height as u32 / 4) as u16,
        "the graphics window that used to be subtracted must cover most of the pane, got {spanning:?}"
    );

    let (_, area) = ladder_dialog_area(&s, &machine, FULL.width);
    eprintln!("zork0 dialog_area = {area:?} (full = {FULL:?}, largest graphics rect = {spanning:?})");
    assert_eq!(
        area, FULL,
        "a modal drops the v6 pixel frame, so it centres in the whole pane — not in what the frame left"
    );

    // …and the symptom itself: a dialog of the leader panel's shape is centred
    // and complete, never clipped against a strip at the bottom of the frame.
    let d = app::render::dialog::centered_rect(area, DIALOG_W, DIALOG_H);
    eprintln!("zork0 dialog rect = {d:?}");
    assert_eq!(d.height, DIALOG_H, "the dialog keeps its full height (its buttons are on the last row)");
    assert_eq!(d.width, DIALOG_W, "the dialog keeps its full width");
    assert!(d.bottom() <= FULL.bottom(), "the dialog's bottom edge is inside the pane");
    assert_eq!(
        (d.y - FULL.y, FULL.bottom() - d.bottom()),
        (FULL.height.saturating_sub(DIALOG_H) / 2, FULL.height.saturating_sub(DIALOG_H).div_ceil(2)),
        "vertically centred in the pane"
    );
}

/// The comparison that says this is v6-specific: a plain Z-machine story at the
/// same terminal was always centred, and stays so.
#[test]
fn non_v6_story_dialog_area_is_the_whole_pane_too() {
    let Some((mut s, machine)) = boot("minizork-r34-s871124.z3", None) else {
        return;
    };
    drive_to_gameplay(&mut s);

    // NON-VACUITY: a v3 story is NOT a Layered composite, which is exactly why it
    // never lost its centring.
    let model = s.screen();
    assert!(
        !matches!(&model.root, WinNode::Layered(_)),
        "Mini-Zork I is a plain Z-machine story, not a v6 composite"
    );

    let (_, area) = ladder_dialog_area(&s, &machine, FULL.width);
    eprintln!("minizork dialog_area = {area:?}");
    assert_eq!(area, FULL, "a non-v6 story's dialog area is the whole pane");
    let d = app::render::dialog::centered_rect(area, DIALOG_W, DIALOG_H);
    eprintln!("minizork dialog rect = {d:?}");
    assert_eq!(d, Rect::new(11, 9, DIALOG_W, DIALOG_H), "centred, whole, and the same rect the v6 case gets");
}

/// The other side of the line, and why the `Layered` arm FILTERS rather than
/// returns nothing: Journey's Amiga floppy (release 30 / serial 890322) puts its
/// illustration in a chrome graphics window entirely BESIDE the story box, and
/// the cell path a modal forces still places that column through the image
/// protocol — "story content, not frame art", in its own words. A dialog centred
/// over it would be overpainted, which is the whole of SQ-0203. So it stays
/// excluded; what it must not do is cost the dialog its HEIGHT, which is the
/// symptom SQ-1092 is about.
///
/// A disk image is a different release, not the same story on other media: the
/// bare `journey-r83-s890706.z6` reaches its gameplay frame with no such column
/// at all (measured: no chrome graphics window beside the story), so it cannot
/// stand in for this.
#[test]
fn journey_side_column_still_pushes_the_dialog_clear_of_it() {
    let Some((mut s, machine)) = boot("Journey - The Quest Begins.adf", Some((30, "890322"))) else {
        return;
    };
    drive_to_gameplay(&mut s);

    // NON-VACUITY: this frame must carry a chrome graphics window entirely left of
    // the story box and alongside its rows — the shape the cell path draws.
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else {
        panic!("Journey must present a v6 Layered root, got {:?}", model.root)
    };
    let story = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .expect("a primary story window");
    let column = items
        .iter()
        .find(|pw| {
            matches!(&pw.node, WinNode::Graphics(g) if g.win != 0)
                && pw.x_px + pw.w_px <= story.x_px
                && pw.y_px < story.y_px + story.h_px
                && pw.y_px + pw.h_px > story.y_px
        })
        .expect("an illustration column beside the story box");
    eprintln!(
        "journey column px = ({}, {}, {}, {}), story px = ({}, {}, {}, {})",
        column.x_px, column.y_px, column.w_px, column.h_px,
        story.x_px, story.y_px, story.w_px, story.h_px,
    );

    let (_, area) = ladder_dialog_area(&s, &machine, FULL.width);
    eprintln!("journey dialog_area = {area:?}");
    assert!(area.x >= column.x + column.w, "the dialog area starts right of the illustration, got {area:?}");
    assert_eq!(area.height, FULL.height, "…and still spans the pane's full height");
    let d = app::render::dialog::centered_rect(area, DIALOG_W, DIALOG_H);
    eprintln!("journey dialog rect = {d:?}");
    assert_eq!(d.height, DIALOG_H, "the dialog keeps its full height and its buttons");
    assert!(d.bottom() <= FULL.bottom(), "the dialog's bottom edge is inside the pane");
}

/// The width at which the two measuring bases came apart — the case none of the
/// others above can see, because 82 columns against an ~80-cell v6 screen is
/// precisely where they coincide.
///
/// `v6_layout::cell_path_side_columns` is now the single statement of which
/// windows the cell path places and where, and both the renderer and
/// `dialog_bounds` call it. This pins the second caller to the first at 160
/// columns: the renderer puts Journey's illustration at pane columns 2..64 there,
/// where the old walk — measuring in the game's own native cells — excluded only
/// 2..33 whatever the pane, leaving the dialog to start at column 33 with thirty-one
/// columns of canyon wall drawn over it by the terminal.
///
/// Asserted as a relation to what the renderer places rather than as a pinned
/// rect, so the two cannot drift apart again; the measured columns are named in
/// the message so a change to the shared rule is still legible in a failure.
#[test]
fn journeys_exclusion_tracks_the_drawn_column_at_a_wide_pane_too() {
    let Some((mut s, machine)) = boot("Journey - The Quest Begins.adf", Some((30, "890322"))) else {
        return;
    };
    drive_to_gameplay(&mut s);
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else {
        panic!("Journey must present a v6 Layered root, got {:?}", model.root)
    };
    let state = ladder_state(&machine);
    let layout = app::render::v6_layout::classify_windows(items, state.v6_text.cell());
    let (native_w, _) = app::render::v6_layout::native_extent(items, &state.v6_text);
    eprintln!("journey native_w = {native_w}");

    for (pane_w, want) in [(82u16, (2u16, 33u16)), (160, (2, 64))] {
        let (full, area) = ladder_dialog_area(&s, &machine, pane_w);
        let story = Rect::new(1, 1, pane_w - 2, FULL.height - 3);
        let cols = app::render::v6_layout::cell_path_side_columns(&layout, story, native_w);
        assert_eq!(cols.len(), 1, "one illustration column at {pane_w} columns");
        let drawn = (cols[0].x, cols[0].x + cols[0].w);
        eprintln!("journey @{pane_w}: drawn column {drawn:?}, dialog_area {area:?}");
        assert_eq!(drawn, want, "the cell path places the illustration at these pane columns");
        assert!(
            area.x >= drawn.1,
            "at {pane_w} columns the dialog area must start at or right of the DRAWN column \
             {drawn:?}, got {area:?} — the two measuring bases have drifted apart again"
        );
        assert_eq!(area.height, full.height, "…and the dialog still gets the pane's full height");
    }
}
