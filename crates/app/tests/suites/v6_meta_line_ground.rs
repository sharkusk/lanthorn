//! SQ-0954 — lanthorn's OWN annotation lines sit on the story window's page, not
//! on the machine's.
//!
//! **Reported by eye**: *"our info messages printed in story output (e.g. after a
//! `|` command) are not printed with the current story background, instead use the
//! machine background, which doesn't work well for Zork Zero."*
//!
//! # Measured cause
//!
//! `period::painted` gives three selectors the machine's PAGE — `transcript_input`
//! (the echoed command), `transcript_meta` (the gutter line) and
//! `transcript_warning`. That is deliberate and, for a v1–v5 story, right: those
//! lines are lanthorn's, their ink says something no machine has an opinion about,
//! but leaving their ground alone punches the host theme's page through the
//! machine's in the middle of the transcript.
//!
//! In v6 the machine's page need not be the ground. A game that calls `set_colour`
//! on window 0 declares its own, and Zork Zero does. Measured at 120x45, colours
//! honoured, with each medium's own period look applied:
//!
//! | medium                    | story page          | period page       | meta line's text sat on |
//! |---------------------------|---------------------|-------------------|-------------------------|
//! | `zork0-r393-s890714.z6`   | `Rgb(173, 173, 173)`| `Rgb(0, 0, 173)`  | **the period page**     |
//! | Amiga floppy (r366)       | `Rgb(173, 173, 173)`| `Rgb(7, 75, 161)` | **the period page**     |
//! | Macintosh disk (r296)     | *(none declared)*   | `Rgb(255,255,255)`| the period page — correct |
//!
//! The gutter GLYPH kept the story's grey throughout, because the symbol cell is
//! drawn with the marker style rather than the line style — so the reported row was
//! two grounds wide, a machine-coloured stripe with a story-coloured pip at its
//! left edge.
//!
//! **The Macintosh is why one machine is not enough.** There the story window
//! declares nothing, the machine's white IS the ground, and the frame is correct
//! before and after. A suite pinned to the reported title on its most obvious
//! medium would have shown no defect at all.
//!
//! # What is pinned
//!
//! The RELATIONSHIP, not a colour constant: **an annotation line is read on the
//! same ground as the prose above it**. Both `honor_game_colours` modes, per
//! CLAUDE.md — with colours declined the game never calls `set_colour`, so there is
//! no story page, the machine's is the ground, and nothing may move.
//!
//! The story media are gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The bare IBM PC story file: Zork Zero r393/890714, which boots
/// `set_colour(fg = 2 black, bg = 9 white)` on window 0 — so its story page is its
/// own and its period look is the IBM PC's blue. The widest gap of the three.
const IBM_PC_STORY: &str = "zork0-r393-s890714.z6";
/// The Amiga release floppy — a different build, r366/890323, and a different
/// machine page again.
const AMIGA_FLOPPY: &str = "Zork Zero - The Revenge of Megaboz.adf";
/// The Macintosh release disk, r296/881019: Zork Zero never calls `set_colour` on
/// it at all, so this is the CONTROL — the machine's page is the ground and the
/// frame must not move.
const MAC_DISK: &str = "Zork Zero Disk.image";
/// **Arthur's Amiga floppy, release 54 / serial 890606** — the second half of the
/// report, and the press that shows why the story page alone is the wrong key.
///
/// Here §8.3's Amiga pair publishes `Rgb(66, 66, 66)` as the ground while the
/// PERIOD LOOK's page is `Rgb(7, 75, 161)`; the two are separate facts and this is
/// where they disagree. Arthur declares no window page of its own, so a fix keyed
/// on `v6_story_page` does nothing here and the meta line stayed a blue sentence
/// in a grey row.
///
/// It needs driving: at the boot frame window 0 is an intro plate, not a
/// transcript, so [`frame`] walks the intro before rendering.
const ARTHUR_FLOPPY: &str = "Arthur - The Quest for Excalibur.adf";

/// The text a case pushes as lanthorn's own, distinctive enough to find by eye in
/// a failure message.
const META: &str = "METAPROBE lanthorn speaking";

/// A pane roomy enough for the chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

struct Frame {
    state: AppState,
    buf: Buffer,
    viewport: Rect,
}

/// Boot `file` the way `startup.rs` does — the medium picks the profile, the
/// archive has its say about colours, and the screen size comes off the mount —
/// then apply that machine's PERIOD LOOK, push one meta line, and render a hybrid
/// frame.
///
/// The period look is the whole point: without it these three selectors carry no
/// background at all and the cells keep whatever the pane painted, which is already
/// the story page. The defect only exists once a machine's page has been laid on
/// them, which is what `period::apply_to_theme` does on every real launch of a
/// licensed medium.
///
/// `None` when the gitignored medium is absent.
fn frame(file: &str, honor: bool) -> Option<Frame> {
    frame_driven(file, honor, false)
}

/// [`frame`], optionally walking the intro first — see [`ARTHUR_FLOPPY`].
fn frame_driven(file: &str, honor: bool, drive: bool) -> Option<Frame> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, PictureOverride::Unset.flavour(), None);
    let mut picts = PictSource::resolve_with_override(&path, PictureOverride::Unset, None);
    let picture_dims = picts.all_pict_dims();
    let honoured = honor && !picts.declines_game_colours(profile.default_colours());
    // SQ-1021/SQ-1022: every per-machine fact in one value, so this
    // harness cannot omit one — it was omitting the CELL.
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
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

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    // Zork Zero's boot banner IS the transcript; Arthur opens on an intro plate
    // and window 0 only becomes a transcript once the intro is walked. Driving to
    // a frame the app actually draws is the whole point — a harness that rendered
    // the plate would measure a screen with no prose on it and agree with itself.
    if drive {
        for _ in 0..14 {
            let r = match session.pending_input() {
                app::session::InputKind::Line => session.submit(""),
                app::session::InputKind::Char => session.submit_char(13),
                app::session::InputKind::Event => session.submit(""),
            };
            if r.transcript.to_lowercase().contains("y or n") {
                let _ = session.submit_char(b'n');
            }
            assert!(!session.quit, "{file}: quit while walking the intro");
        }
        let r = session.submit("look");
        assert!(r.fault.is_none(), "{file}: `look` faulted: {:?}", r.fault);
    }
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);
    state.push_transcript_kind(META, TranscriptKind::Meta);

    // The machine's own screen, laid under the theme — `reload.rs` does exactly
    // this on every launch whose medium licenses it.
    if let Some(look) = app::period::resolve(profile, true, honoured, true, Some(6)) {
        app::period::apply_to_theme(&mut state.colors.theme, &look, Some(6));
        state.period_look = Some(look);
    }

    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    let viewport = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    Some(Frame { state, buf, viewport })
}

/// Every background on the first row containing `needle` — the cells with a glyph
/// in them AND the blank ones beside them, which is the comparison that matters:
/// the reported symptom is a coloured SENTENCE in a differently-coloured ROW.
///
/// One entry means one ground across the whole row, which is what a line printed
/// on a page looks like.
fn whole_row_grounds(f: &Frame, needle: &str) -> Option<Vec<Color>> {
    let y = row_of(f, needle)?;
    let mut out: Vec<Color> = Vec::new();
    for x in f.viewport.x..f.viewport.right() {
        let bg = f.buf.cell((x, y)).expect("in-bounds cell").bg;
        if !out.contains(&bg) {
            out.push(bg);
        }
    }
    Some(out)
}

/// The first row containing `needle`.
fn row_of(f: &Frame, needle: &str) -> Option<u16> {
    let text = |y: u16| -> String {
        (f.viewport.x..f.viewport.right())
            .map(|x| f.buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    (f.viewport.y..f.viewport.bottom()).find(|&y| text(y).contains(needle))
}

/// The backgrounds carried by the drawn (non-blank) cells of the first row
/// containing `needle`, most common first. `None` when no row has it.
fn row_grounds(f: &Frame, needle: &str) -> Option<Vec<Color>> {
    let text = |y: u16| -> String {
        (f.viewport.x..f.viewport.right())
            .map(|x| f.buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    let y = (f.viewport.y..f.viewport.bottom()).find(|&y| text(y).contains(needle))?;
    let mut out: Vec<Color> = Vec::new();
    for x in f.viewport.x..f.viewport.right() {
        let c = f.buf.cell((x, y)).expect("in-bounds cell");
        if c.symbol() != " " && !out.contains(&c.bg) {
            out.push(c.bg);
        }
    }
    Some(out)
}

/// **The deliverable, as a relation.** On every medium and in both colour modes,
/// lanthorn's own meta line is read on ONE ground — the one the pane put down —
/// and not as a coloured sentence sitting in a differently-coloured row.
///
/// The row is its own reference, which is what lets one case cover four presses.
/// Zork Zero prints prose into window 0 and the room name is a second reference
/// (asserted below where it exists); Arthur's transcript is EMPTY on this frame —
/// its prose never reaches `take_transcript_elems` at all, which is a separate
/// defect and filed as one — so a case keyed on a prose row would have had nothing
/// to compare against on the very press the second half of the report came from.
///
/// Falsified by reverting `render::transcript`'s re-grounding: Zork Zero r393 comes
/// back `[Rgb(173, 173, 173), Rgb(0, 0, 173)]` and Arthur's Amiga floppy
/// `[Rgb(66, 66, 66), Rgb(7, 75, 161)]` — in both, the machine's or the period
/// look's page laid over the page the frame is actually being read on.
#[test]
fn a_meta_line_is_read_on_one_ground_and_it_is_the_frames_own() {
    let mut seen = 0usize;
    let mut any = false;
    for (file, drive) in [
        (IBM_PC_STORY, false),
        (AMIGA_FLOPPY, false),
        (MAC_DISK, false),
        (ARTHUR_FLOPPY, true),
    ] {
        any |= stories_dir().join(file).exists();
        for honor in [true, false] {
            let Some(f) = frame_driven(file, honor, drive) else { continue };
            seen += 1;
            let what = format!("{file} honor={honor}");

            let meta = whole_row_grounds(&f, META)
                .unwrap_or_else(|| panic!("{what}: premise — the meta line reached the frame"));
            assert_eq!(
                meta.len(),
                1,
                "{what}: lanthorn's own line is a coloured sentence in a differently-coloured \
                 row — the machine's or the period look's page laid over the frame's own: {meta:?}",
            );

            // …and where the game printed prose, that is the same ground. The
            // stronger statement, available on the presses that have a room name.
            if let Some(prose) = row_grounds(&f, "Banquet Hall") {
                assert_eq!(
                    prose.len(),
                    1,
                    "{what}: premise — the prose row is one ground, or it is no reference: {prose:?}",
                );
                assert_eq!(
                    meta, prose,
                    "{what}: the meta line is read on a different ground from the prose above it",
                );
            }
        }
    }
    assert!(!any || seen > 0, "a present medium must have produced a frame");
}

/// **The direction**, and what stops the case above passing vacuously: the machine
/// really does lay a page on these selectors, and on the two media where the story
/// declares its own it is a DIFFERENT colour. Without this, a build that stopped
/// applying period looks at all would satisfy the relation trivially.
#[test]
fn the_period_look_really_does_paint_a_page_of_its_own() {
    let mut seen = 0usize;
    let mut any = false;
    for (file, declares_its_own) in [(IBM_PC_STORY, true), (AMIGA_FLOPPY, true), (MAC_DISK, false)] {
        any |= stories_dir().join(file).exists();
        let Some(f) = frame(file, true) else { continue };
        seen += 1;
        let look = f.state.period_look.expect("a licensed medium at v6 has a period look");
        let page = Color::Rgb(look.page.0, look.page.1, look.page.2);
        let story = f.state.v6_story_page.get();
        assert_eq!(
            story.is_some(),
            declares_its_own,
            "{file}: premise — whether the story window declares a page of its own",
        );
        if let Some((r, g, b)) = story {
            assert_ne!(
                Color::Rgb(r, g, b),
                page,
                "{file}: premise — the two pages must DIFFER here, or this medium proves nothing",
            );
        }
    }
    assert!(!any || seen > 0, "a present medium must have produced a frame");
}
