//! SQ-0848 — an inline story float is flattened onto the MACHINE's page when the
//! story window declares none of its own.
//!
//! **Reported by eye**, immediately after SQ-0846 landed, on `stories/Zork Zero
//! Disk.image` — **Zork Zero release 296 / serial 881019**, the Macintosh disk —
//! with the disk's own default (colour) archive: *"the room icon background is
//! terminal default, rather than the white story pane background"*.
//!
//! # Measured cause
//!
//! An inline story picture is handed to the image protocol WITH ITS ALPHA, and
//! kitty composites that alpha against the TERMINAL's background rather than the
//! cells underneath — so `render::inline_image::flatten_onto` resolves it for the
//! terminal, against a page `page_for` picks. That page had two layers: the story
//! window's EXPLICIT background (`AppState::v6_story_page`, published only from a
//! colour the game set with `set_colour`), else the theme's `inline_image` style.
//!
//! Zork Zero on the Macintosh **never calls `set_colour` at all** — measured, and
//! stated on `session::machine_screen_pair`: on both of the disk's archives and in
//! both colour modes every window stays `ZColour::Default`. So layer 1 is `None`
//! on every frame of it and the floats fell through to layer 2. Measured at 120x45
//! on a halfblocks picker, release 296 with `CPic.data`:
//!
//! | link                     | before SQ-0848             | after                  |
//! |--------------------------|----------------------------|------------------------|
//! | `v6_story_page`          | `None`                     | `None` (unchanged)     |
//! | `v6_page_pair` → page    | `Rgba([255, 255, 255, …])` | same                   |
//! | resolved float page      | **`Rgba([0, 0, 0, …])`**   | `Rgba([255, 255, 255])`|
//! | room-icon ground on row 20 | `Rgb(0, 0, 0)`           | `Rgb(255, 255, 255)`   |
//!
//! That black is the theme's `chrome`, which `inline_image` inherits with an empty
//! delta — the icons were flattened, just onto the wrong ground, while the pane
//! around them was the machine's white. SQ-0846 is what made the difference
//! visible: before it, nothing painted the Macintosh's `$2C`/`$2D` pair and the
//! whole pane was the theme's dark too.
//!
//! The fix gives the ground a middle layer: the window's explicit page, else the
//! MACHINE's (`AppState::v6_page_pair`), else the theme — the same layering
//! `screen::v6_host_pair` already performs for the pixel ring, and
//! `screen::v6_machine_page` for the transcript's own cells.
//!
//! # What is pinned
//!
//! The RELATIONSHIP, never a colour constant: **the ground a float is flattened
//! onto is the ground the prose beside it is read on** (`v6_float_margin_ground`'s
//! header says why — a hardcoded RGB does not survive a palette or theme change).
//! It is asserted twice per frame, at both ends of the boundary the defect lives
//! at: on the page `render::inline_image::float_page` resolves, and on the strip
//! actually drawn, where the picture's own transparent ground must come out as
//! that page.
//!
//! Both `honor_game_colours` modes are pinned. With the game's colours declined
//! the machine publishes no pair at all, so the new layer is structurally absent
//! and the frame is byte-identical to the two-layer behaviour that shipped.
//!
//! The story media are gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The Macintosh release disk the defect was reported on: Zork Zero r296/881019.
const MAC_DISK: &str = "Zork Zero Disk.image";
/// The Amiga release floppy: a DIFFERENT build, r366/890323.
const AMIGA_FLOPPY: &str = "Zork Zero - The Revenge of Megaboz.adf";
/// The bare IBM PC story file: a THIRD build, r393/890714.
const IBM_PC_STORY: &str = "zork0-r393-s890714.z6";

/// A pane roomy enough for the chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One frame, measured: the state the render published and the buffer it drew.
struct Frame {
    state: AppState,
    buf: Buffer,
    viewport: Rect,
}

/// Boot `file` exactly as `startup.rs` does — the medium picks the profile, the
/// archive has its say about colours — accumulate the boot banner through the real
/// elems pipeline so `transcript_images` carries the window-0 floats (Zork Zero's
/// ornate drop-cap and its small room icon) as a live session would, and render
/// one hybrid frame.
///
/// `None` when the gitignored medium is absent.
fn frame(file: &str, pictures: Option<&str>, honor: bool) -> Option<Frame> {
    frame_at(file, pictures, honor, PANE)
}

/// [`frame`] at a stated pane, so a case can move the v6 magnification without
/// moving anything else (SQ-1002).
fn frame_at(file: &str, pictures: Option<&str>, honor: bool, pane: Rect) -> Option<Frame> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    let dir = app::scratch_dir("sq848");
    let over = match pictures {
        Some(name) => PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => PictureOverride::Unset,
    };
    let named_art_std_window = over.std_window();
    let profile = InterpreterProfile::resolve(&path, None, over.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();
    let honoured = honor
        && !picts.declines_game_colours(profile.default_colours());
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        named_art_std_window,
        profile.interpreter_number(),
        honoured.then(|| profile.default_colours()).flatten(),
        true,
        app::native_font::FaceSet::none(),
        profile.palette(),
        None,
    );
    let mut session = GameSession::new_for_machine(bytes, honoured, false, false, picture_dims, None, None, &boot)
    .expect("Zork Zero should load and boot without a ZError");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);

    let model = session.screen();
    let mut buf = Buffer::empty(pane);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, pane, &mut buf);
    let viewport = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    Some(Frame { state, buf, viewport })
}

/// One row where prose flows beside a picture: the row, the column the prose
/// starts at, and the ground that prose is read on.
///
/// Recognised from the frame alone, exactly as `v6_float_margin_ground` does it,
/// so the test never re-implements the wrap's geometry: the prose starts at some
/// column `t > 0` and at least one column left of `t` carries a glyph — the
/// picture, drawn as halfblock cells. That excludes an ordinary paragraph indent
/// (all-blank leading columns) and a band row (no prose at all).
struct FloatRow {
    row: u16,
    prose_col: u16,
    prose_bg: ratatui::style::Color,
}

fn float_rows(f: &Frame) -> Vec<FloatRow> {
    let vp = f.viewport;
    let glyph = |x: u16, y: u16| {
        f.buf.cell((x, y)).and_then(|c| c.symbol().chars().next()).unwrap_or(' ')
    };
    let mut out = Vec::new();
    for y in vp.y..vp.bottom() {
        let Some(t) = (vp.x..vp.right()).find(|&x| glyph(x, y).is_alphanumeric()) else { continue };
        if t == vp.x || !(vp.x..t).any(|x| glyph(x, y) != ' ') {
            continue;
        }
        out.push(FloatRow { row: y, prose_col: t, prose_bg: f.buf.cell((t, y)).unwrap().bg });
    }
    out
}

/// **The deliverable, in one relation.** Assert that every float on this frame is
/// flattened onto the ground its own prose is read on — at the page the render
/// resolved, and in the pixels it drew.
///
/// The corroboration that the strip was really flattened onto that page lives in
/// [`the_float_strip_is_flattened_onto_the_page_the_frame_resolved`], as a
/// DIFFERENCE rather than as a colour count.
///
/// It used to be counted here: cells of the picture whose halfblock pair was the
/// page in BOTH halves. That worked only because the picture's box was loose —
/// `fit_preserving_aspect` letterboxes into a ceil'd cell box, and the leftover
/// margin is transparent ground. SQ-1002 sized these pictures at the TEXT's rate
/// instead of the chrome ring's, which is both correct and tighter: the box is now
/// the footprint the game drew the picture for, so at the Macintosh's two-colour
/// archive not one cell is entirely margin any more (measured: 2 such cells on
/// three of the four frames, 0 on the fourth). An oracle that depends on how much
/// slack a resample happens to leave is luck, not evidence — so the claim moved to
/// a test that changes the page and requires the picture to change with it.
fn assert_float_ground_is_the_prose_ground(f: &Frame, what: &str) {
    let rows = float_rows(f);
    assert!(
        rows.len() >= 4,
        "{what}: premise — the boot banner really does float prose beside its drop-cap and its \
         room icon (found {} such rows)",
        rows.len()
    );
    let page = app::render::inline_image::float_page(&f.state)
        .unwrap_or_else(|| panic!("{what}: this frame resolves no page at all for its inline floats"));
    let page_color = ratatui::style::Color::Rgb(page[0], page[1], page[2]);

    for r in &rows {
        eprintln!(
            "{what}: float row {} prose@{} prose_bg={:?} float_page={:?}",
            r.row, r.prose_col, r.prose_bg, page
        );
        assert_eq!(
            r.prose_bg, page_color,
            "{what}: row {}: the ground the float is flattened onto must be the ground the prose \
             beside it is read on",
            r.row
        );
    }
}

// ── The Macintosh: the reported case ─────────────────────────────────────────

/// **THE DELIVERABLE — SQ-0848, as reported**, on `stories/Zork Zero Disk.image`
/// (release 296 / serial 881019) with the disk's own default colour archive, and
/// again on the two-colour one, because a Macintosh is one machine with two
/// archives and the page belongs to the machine.
///
/// FALSIFIED by dropping the machine layer from
/// `render::inline_image::page_for`: the resolved page becomes the theme's
/// `Rgba([0, 0, 0, 255])` against a prose ground of `Rgb(255, 255, 255)` and both
/// halves fail — *"the room icon background is terminal default, rather than the
/// white story pane background"*, verbatim.
#[test]
fn the_macintosh_float_ground_is_the_machines_white_page() {
    for archive in [None, Some("Pic.data")] {
        let Some(f) = frame(MAC_DISK, archive, true) else { return };
        let what = format!("mac r296 {archive:?}");

        // The two premises that make this case non-vacuous, and the whole reason
        // the theme was reached at all: the game names no colour for its own
        // window, while the machine names the page under it.
        assert_eq!(
            f.state.v6_story_page.get(),
            None,
            "{what}: premise — Zork Zero never calls set_colour on the Macintosh, so the story \
             window publishes no page of its own",
        );
        assert!(
            f.state.v6_page_pair.get().is_some(),
            "{what}: premise — the Macintosh publishes a screen pair (SQ-0846)",
        );
        assert_eq!(
            app::render::screen::v6_host_pair(&f.state).1,
            image::Rgba([255, 255, 255, 255]),
            "{what}: and that pair's page is the Macintosh's white",
        );

        assert_float_ground_is_the_prose_ground(&f, &what);
    }
}

/// **Corroboration, as a DIFFERENCE: the strip really is flattened onto the page
/// the frame resolved** (SQ-1002 replaces SQ-0848's colour count; see
/// [`assert_float_ground_is_the_prose_ground`]).
///
/// A resolved page proves what the render DECIDED, never what it drew. The one
/// thing that separates the two is whether the picture's own pixels move when the
/// page does: `flatten_onto` composites the cut-out's transparent ground against
/// the page BEFORE the strip is encoded, so a different page is a different strip
/// and different cells. A strip that was never flattened encodes the same raw
/// artwork either way and comes out byte-identical — which is exactly the reported
/// defect, a room icon sitting on the terminal's ground instead of the story's.
///
/// The same medium at `honor_game_colours` true and false is the pair to use,
/// because the switch is what moves the page and nothing else about these frames:
/// true resolves the Macintosh's white through the machine layer, false leaves the
/// theme's dark ground (`the_macintosh_machine_page_never_survives_declined_colours`
/// pins that half). Both archives, because a Macintosh is one machine with two.
///
/// Non-vacuous in both directions: the pages must actually differ, and both frames
/// must actually float something.
#[test]
fn the_float_strip_is_flattened_onto_the_page_the_frame_resolved() {
    for archive in [None, Some("Pic.data")] {
        let (Some(honoured), Some(declined)) =
            (frame(MAC_DISK, archive, true), frame(MAC_DISK, archive, false))
        else {
            return;
        };
        let what = format!("mac r296 {archive:?}");
        assert_ne!(
            app::render::inline_image::float_page(&honoured.state),
            app::render::inline_image::float_page(&declined.state),
            "{what}: premise — the two frames must resolve DIFFERENT pages, or this proves nothing",
        );

        // Every cell of every float's own columns, keyed by position so a layout
        // that moved would show up as a difference rather than hide one.
        let strip = |f: &Frame| -> Vec<(u16, u16, String, ratatui::style::Color, ratatui::style::Color)> {
            float_rows(f)
                .iter()
                .flat_map(|r| {
                    (f.viewport.x..r.prose_col.saturating_sub(1)).map(move |x| {
                        let c = f.buf.cell((x, r.row)).unwrap();
                        (x, r.row, c.symbol().to_string(), c.fg, c.bg)
                    })
                })
                .collect()
        };
        let (a, b) = (strip(&honoured), strip(&declined));
        assert!(!a.is_empty(), "{what}: premise — the honoured frame floats a picture at all");
        assert!(!b.is_empty(), "{what}: premise — so does the declined one");
        assert_ne!(
            a, b,
            "{what}: the pictures' cells are identical under two different pages, so the strip was \
             never composited against either — a cut-out handed to the protocol with its alpha \
             intact draws the same pixels whatever the story window declared",
        );
    }
}

/// **SQ-1002: a story picture is sized by the TEXT it sits beside, never by the
/// chrome ring's magnification.**
///
/// Zork Zero draws its ornate drop-cap four of its own text lines tall, on a
/// screen whose character cell is 8x16 native pixels. Hybrid maps that space onto
/// the terminal at two rates at once — art by the letterbox factor `s`, text at
/// one native cell per terminal cell — and these pictures used to be scaled by
/// `s`, "to match the chrome ring". They sit in the TEXT, so at `s = 2` the cap
/// claimed eight terminal rows beside the four-row paragraph it opens, and it grew
/// further the bigger the pane got. The real DOS press
/// (`machine-screenshots/dos-zorkzero.png`) shows the cap spanning exactly four
/// lines and the room icon three.
///
/// Two panes of the SAME WIDTH — so the wrap is identical and only the
/// magnification moves — must indent their prose by the same number of columns,
/// because that indent is the picture's own width plus the game's margin. Under
/// the old rule it moved with `s`.
///
/// Non-vacuous by its premise: the two frames must actually resolve different
/// magnifications, or the case is comparing a frame with itself.
#[test]
fn a_float_keeps_its_footprint_when_the_art_magnification_moves() {
    const TALL: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };
    const SHORT: Rect = Rect { x: 0, y: 0, width: 120, height: 25 };
    let (Some(tall), Some(short)) = (
        frame_at(IBM_PC_STORY, None, true, TALL),
        frame_at(IBM_PC_STORY, None, true, SHORT),
    ) else {
        return;
    };
    let (st, ss) = (tall.state.v6_image_scale.get(), short.state.v6_image_scale.get());
    assert_ne!(st, ss, "premise: the two panes must magnify the ART differently ({st} vs {ss})");

    let indent = |f: &Frame| {
        let rows = float_rows(f);
        assert!(!rows.is_empty(), "premise: the boot banner floats prose beside its drop-cap");
        rows[0].prose_col - f.viewport.x
    };
    assert_eq!(
        indent(&tall),
        indent(&short),
        "the drop-cap's footprint moved with the chrome ring's magnification ({st}x vs {ss}x) — a \
         picture drawn inside the text flow is sized by the text, which is one native 8x16 cell \
         per terminal cell whatever the art is scaled to",
    );
}

/// **Guard: `honor_game_colours = false` stays a no-op.**
///
/// The colour bit gates `session::machine_screen_pair` at the source, so a
/// colourless interpreter is never handed the profile's pair and `v6_page_pair` is
/// `None` — the new middle layer is structurally absent and the frame resolves its
/// float ground through exactly the two layers that shipped before. The machine's
/// white must not reach it by any other route.
#[test]
fn the_macintosh_machine_page_never_survives_declined_colours() {
    for archive in [None, Some("Pic.data")] {
        let Some(f) = frame(MAC_DISK, archive, false) else { return };
        let what = format!("mac r296 {archive:?} declined");
        assert!(
            f.state.v6_page_pair.get().is_none(),
            "{what}: colours declined — there is no machine pair to lay down",
        );
        assert_eq!(f.state.v6_story_page.get(), None, "{what}: nor a story page");
        assert_ne!(
            app::render::inline_image::float_page(&f.state),
            Some(image::Rgba([255, 255, 255, 255])),
            "{what}: the Macintosh's white must not smuggle itself past the switch that declined it",
        );
    }
}

// ── The controls ─────────────────────────────────────────────────────────────

/// **The IBM PC is the CONTROL, not merely a regression risk** (SQ-0827 used the
/// same one, and the user confirmed it by eye).
///
/// `zork0-r393-s890714.z6` is a THIRD build — release 393 / serial 890714 — and
/// its profile publishes no machine pair at all, so nothing may move here. It also
/// settles the other half: r393 boots `set_colour(fg=2 black, bg=9 white)` on
/// window 0, so its float ground comes from layer 1 and is untouched by a layer
/// that does not exist on this frame.
#[test]
fn the_ibm_pc_control_has_no_machine_page_to_gain() {
    let Some(f) = frame(IBM_PC_STORY, None, true) else { return };
    assert!(
        f.state.v6_page_pair.get().is_none(),
        "premise: outside the Amiga and the Macintosh no machine pair is published — which is why \
         nothing on this profile could have moved",
    );
    assert!(
        f.state.v6_story_page.get().is_some(),
        "premise: r393 boots set_colour(bg=9 white) on window 0, so layer 1 decides here",
    );
    assert_float_ground_is_the_prose_ground(&f, "ibmpc r393");
}

/// **An explicit window background still beats the machine's page.**
///
/// The Amiga floppy is the one medium where BOTH layers are live: `Zork Zero - The
/// Revenge of Megaboz.adf` is release 366 / serial 890323, it declares its own
/// light-grey window-0 page, and §8.3's Amiga interpreter publishes a dark-grey
/// screen pair underneath it. The window's own page must win — the frame is
/// byte-identical to the one that shipped before SQ-0848 — and the two really are
/// different colours, or this case would prove nothing.
#[test]
fn an_explicit_window_page_still_beats_the_machines() {
    let Some(f) = frame(AMIGA_FLOPPY, None, true) else { return };
    let story = f.state.v6_story_page.get().expect("Zork Zero declares its own window-0 page");
    let machine = app::render::screen::v6_host_pair(&f.state).1;
    assert!(
        f.state.v6_page_pair.get().is_some(),
        "premise: §8.3's Amiga interpreter publishes a screen pair",
    );
    assert_ne!(
        (story.0, story.1, story.2),
        (machine[0], machine[1], machine[2]),
        "premise: the window's page and the machine's really are different colours here",
    );
    assert_eq!(
        app::render::inline_image::float_page(&f.state),
        Some(image::Rgba([story.0, story.1, story.2, 255])),
        "the window the picture floats in named a page; the machine's ground stays under it",
    );
    assert_float_ground_is_the_prose_ground(&f, "amiga r366");
}
