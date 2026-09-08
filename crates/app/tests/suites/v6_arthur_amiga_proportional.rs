//! SQ-1009 — Arthur's Amiga floppy is drawn with the face the disk carries, at that
//! face's own advances and on its own line.
//!
//! # What the machine does that lanthorn did not
//!
//! `machine-screenshots/amiga-arthur-*.png` are captures of Arthur running on a real
//! Amiga, all exactly 2x the machine's 320x200 frame (they carry the overscan
//! border, so an absolute x in one of them is 43 px right of the same pixel in our
//! 640x400 native space — see `info.txt`). Every one of them shows PROPORTIONAL
//! text: `i` narrow, `m` wide, and a line pitch of 20 device px where lanthorn gave
//! the story 16.
//!
//! Both facts come off the disk. `char.data` is a 10x10 proportional AmigaDOS font
//! with a real advance per glyph, and Arthur's press doubles its 320-wide art onto
//! the 640x400 unit screen — so one face row is two native rows and the DECLARED
//! cell is 8x20 rather than the machine table's 8x16.
//!
//! # The three things this pins, and why each needs its own case
//!
//! 1. **The declared cell follows the admitted face.** The story is told a 20-row
//!    line, which is what makes it lay out 20 rows where it used to lay out 25.
//! 2. **The pen advances per glyph.** `native_disk_font.rs` already pins the FONT's
//!    advance table against three runs measured in `amiga-arthur-text.png`
//!    (628/634/204 device px of ink). This suite asserts the same three numbers one
//!    layer out — against the pixels the RASTER path actually composites — because
//!    a correct table and a renderer that ignores it look identical from inside the
//!    font.
//! 3. **A grid publishes one run per character, and the ENGINE holds the pen.**
//!    Arthur's score bar arrives as 73 single-character runs, each where `zvm`'s
//!    own cursor stopped — which is the face's advance, not the declared cell, so
//!    the glyph origins of `Church` step 12, 10, 10, 10, 10 native px exactly as
//!    `amiga-arthur-church.png` shows them. The engine measuring with the same
//!    table the renderer draws with is what also puts the bar's right-hand date
//!    field where the machine put it (the game right-aligns it from header `$30`)
//!    and wraps the F5 description where the machine wraps it (`exec.rs` breaks the
//!    line at the window's real pixel width). **Both backends take that break.**
//!    Hybrid was first given a second wrap of its own, so its grid could fill the
//!    window's 73 declared columns while the runs kept the machine's 584-pixel
//!    lines; two passes cannot agree character for character — each assumes it is
//!    the only one breaking, and they swallow different blanks — so a word landed
//!    on one row carrying the other's cell. One pass in both units now, breaking at
//!    whichever fills first, which leaves hybrid's lines honestly shorter than the
//!    window and identical to the machine's.
//!
//! # Fixtures
//!
//! `stories/` is gitignored, so every case that needs the floppy skips vacuously.
//! The capture-derived numbers are transcribed here rather than measured at test
//! time, so the cases stay honest without the PNGs.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::GameSession;

/// Release 54 / serial 890606 — the Amiga press, and the only release in the tree
/// whose medium carries a proportional typeface.
const FIXTURE: &str = "Arthur - The Quest for Excalibur.adf";
const RELEASE: u16 = 54;
const SERIAL: &str = "890606";

/// The captures are 2x the machine's 320x200 frame, and so is our native space, so
/// a SPAN in one is a span in the other. (An absolute x is not — they carry the
/// overscan border.)
const CAPTURE_IS_NATIVE_SCALE: u32 = 1;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` boots (CLAUDE.md): the medium picks the profile and
/// the source, the archive has its say about the screen, and the release's own face
/// rides along in `MachineBoot` — which is what settles the declared cell now.
fn boot() -> Option<(GameSession, app::machine_boot::MachineBoot)> {
    let path = stories_dir().join(FIXTURE);
    let Ok((loaded, _)) = app::hints::load_mounted_story(&path) else {
        eprintln!("SKIP: gitignored floppy missing at {}", path.display());
        return None;
    };
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "{FIXTURE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), SERIAL, "{FIXTURE}: serial");
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    // The whole cascade, exactly as `startup.rs` asks it — but with `disks: None`,
    // because a case here must not depend on what the person running it happens to
    // keep in `~/.lanthorn/` (SQ-1037). Arthur's floppy answers on the first rung.
    let faces = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: picts.art_scale(),
        disks: None,
    });
    let machine = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        true,
        faces,
        profile.palette(),
        None,
    );
    let mut s =
        GameSession::new_for_machine(bytes, true, false, false, picture_dims, None, None, &machine)
            .unwrap_or_else(|e| panic!("{FIXTURE}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some((s, machine))
}

/// A raster `AppState` carrying the machine's own cell, face and art scale.
fn raster_state(machine: &app::machine_boot::MachineBoot) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.v6_render = app::config::V6RenderMode::Raster;
    state.config.honor_game_colours = true;
    state.v6_text = machine.text_face();
    if let Some(scale) = machine.art_scale {
        state.v6_art_scale = scale;
    }
    state
}

/// Drive the intro to the churchyard — **16 turns and a `look`**, which is the same
/// route `v6_arthur_hint_box` takes. The frame's shape is guarded below rather than
/// assumed.
fn in_the_churchyard() -> Option<(GameSession, app::state::AppState)> {
    let (mut s, machine) = boot()?;
    let mut state = raster_state(&machine);
    let t0 = s.take_transcript();
    state.push_transcript_kind(&t0, app::state::TranscriptKind::Story);
    for _ in 0..15 {
        let r = match s.pending_input() {
            app::session::InputKind::Line => s.submit(""),
            app::session::InputKind::Char => s.submit_char(13),
            app::session::InputKind::Event => s.submit(""),
        };
        state.push_transcript_kind(&r.transcript, app::state::TranscriptKind::Story);
        if r.transcript.to_lowercase().contains("y or n") {
            let r2 = s.submit_char(b'n');
            state.push_transcript_kind(&r2.transcript, app::state::TranscriptKind::Story);
        }
        assert!(!s.quit, "{FIXTURE}: quit while walking the intro");
    }
    let r = s.submit("look");
    assert!(r.fault.is_none(), "{FIXTURE}: `look` faulted: {:?}", r.fault);
    state.push_transcript_kind(&r.transcript, app::state::TranscriptKind::Story);
    Some((s, state))
}

/// The raster composite for the frame `session` is standing on.
fn raster(session: &GameSession, state: &app::state::AppState) -> image::RgbaImage {
    let model = Engine::screen(session);
    let WinNode::Layered(items) = &model.root else { panic!("{FIXTURE}: a v6 frame is Layered") };
    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    app::render::screen::build_v6_raster_canvas(&layout, native, state).0
}

/// The x of every inked column-run in `rows` of `canvas`, against `ground`.
///
/// One entry per glyph on a text row, which is what makes an ADVANCE measurable:
/// the distance between two entries' starts is the pen's step.
fn glyph_runs(
    canvas: &image::RgbaImage,
    rows: std::ops::Range<u32>,
    xs: std::ops::Range<u32>,
    ground: image::Rgba<u8>,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut cur: Option<u32> = None;
    for x in xs.clone() {
        let inked = rows.clone().any(|y| *canvas.get_pixel(x, y) != ground);
        match (inked, cur) {
            (true, None) => cur = Some(x),
            (false, Some(c)) => {
                out.push((c, x - 1));
                cur = None;
            }
            _ => {}
        }
    }
    if let Some(c) = cur {
        out.push((c, xs.end - 1));
    }
    out
}

/// First inked column of the first glyph to last inked column of the last — what a
/// SCREEN shows, one pixel narrower than the pen total because the final glyph's
/// right side bearing leaves no mark. The same model `native_disk_font`'s `ink()`
/// closure uses, so the two layers are measuring the same quantity.
fn ink_span(canvas: &image::RgbaImage, rows: std::ops::Range<u32>, xs: std::ops::Range<u32>, ground: image::Rgba<u8>) -> Option<u32> {
    let runs = glyph_runs(canvas, rows, xs, ground);
    Some(runs.last()?.1 - runs.first()?.0 + 1)
}

// ── 1. the declared cell follows the face ────────────────────────────────────

/// The line the STORY is told is the face's height times the art scale — 8x20.
///
/// Falsified by returning `profile.v6_font_cell()` from `native_font::declared_cell`
/// unconditionally, which is what shipped before SQ-1009: the cell comes back 8x16
/// and the game lays out 25 rows where the machine shows 20.
#[test]
fn the_amiga_floppy_declares_the_faces_own_twenty_row_line() {
    let Some((session, machine)) = boot() else { return };
    let face = machine.faces.body().expect("Arthur's floppy carries char.data");
    assert_eq!((face.width, face.height), (10, 10), "char.data is 10x10 nominal");
    assert!(face.proportional, "…and a TYPEFACE, which is what admits it at all");
    assert_eq!(machine.art_scale, Some((2, 2)), "a 320-wide press doubles onto the unit screen");
    assert_eq!(
        machine.cell,
        zvm::screen::V6Cell::new(8, 20),
        "the height is the face's, times the art scale; the width stays the machine's \
         because a proportional face has no single advance to declare",
    );
    // And the story RECEIVED it — the boot is what makes the declaration real.
    assert_eq!(session.machine.v6_cell(), zvm::screen::V6Cell::new(8, 20), "the engine holds it too");
    assert_eq!(
        u32::from(machine.cell.h()),
        u32::from(face.height) * 2 * CAPTURE_IS_NATIVE_SCALE,
        "20 native rows = the 10 machine rows `amiga-arthur.png` measures, doubled",
    );
}

/// No other configuration in the tree admits a `Metric` face, so nothing else moves.
///
/// Journey's and Beyond Zork's Amiga `Char.data` is the font-3 SET — fixed 8x8,
/// code 65 a solid block (SQ-1017) — and a fixed face is either the cell or nothing.
#[test]
fn only_a_proportional_face_moves_a_machines_cell() {
    for (disk, profile) in [
        ("Journey - The Quest Begins.adf", InterpreterProfile::Amiga),
        ("Beyond Zork - The Coconut of Quendor.adf", InterpreterProfile::Amiga),
    ] {
        let path = stories_dir().join(disk);
        if !path.is_file() {
            eprintln!("SKIP: gitignored floppy absent: {disk}");
            continue;
        }
        let (p, source) = InterpreterProfile::resolve_with_source(&path, None, None, None);
        let faces = app::native_font::resolve(&app::native_font::FaceRequest {
            story_path: &path,
            entry: None,
            profile: p,
            source,
            art_scale: Some((2, 2)),
            disks: None,
        });
        assert_eq!(
            app::native_font::declared_cell(profile, &faces, (2, 2)),
            profile.v6_font_cell(),
            "{disk}: a fixed-pitch face declares nothing — the machine's cell stands",
        );
    }
}

// ── 2. the pen, measured on the pixels the raster path composites ────────────

/// The three runs `machine-screenshots/amiga-arthur-text.png` measures, rendered.
///
/// `native_disk_font::the_advance_table_predicts_a_real_amiga_frame_to_the_pixel`
/// asserts these same numbers against the FONT. This asserts them against the
/// COMPOSITE, which is the layer the player sees and the layer that was wrong: a
/// correct advance table and a renderer that steps by a fixed cell are
/// indistinguishable from inside the font.
///
/// The prose is pushed onto the transcript directly rather than driven out of the
/// game, so the case measures the three runs the capture measures rather than
/// whatever line the intro happens to leave on screen.
#[test]
fn the_raster_prose_draws_the_runs_the_capture_measures() {
    let Some((_session, machine)) = boot() else { return };
    let tf = machine.text_face();
    assert!(tf.proportional(), "non-vacuity: the pen under test is the face's");

    let ground = image::Rgba([0, 0, 0, 255]);
    let ink = image::Rgba([255, 255, 255, 255]);
    for (run, measured_at_2x) in [
        ("WHOSO PULLETH OUT THIS SWORD OF THIS STONE, IS RIGHTWISE KING", 628u32),
        ("You are shivering in the cold night air of an English churchyard, unsure", 634),
        ("BORN OF ALL ENGLAND.", 204),
    ] {
        let mut state = raster_state(&machine);
        state.push_transcript_kind(run, app::state::TranscriptKind::Story);
        // A box wide enough that the wrap cannot break the run — the wrap itself is
        // pinned separately below.
        let (cols, rows) = (120u16, 6u16);
        let (mut main, _) = app::render::screen::build_main_text(&state, cols, rows);
        // The live caret is the host's, not the game's, and it is 8 native px of
        // solid block past the end of the run — measure the RUN.
        main.awaiting = false;
        assert_eq!(
            main.lines.iter().filter(|l| !l.is_empty()).count(),
            1,
            "non-vacuity: {run:?} must reach the raster as ONE row, got {:?}",
            main.lines,
        );
        let mut canvas = image::RgbaImage::from_pixel(
            cols as u32 * u32::from(tf.cell().w()),
            rows as u32 * u32::from(tf.cell().h()),
            ground,
        );
        v6::draw_story_text(&mut canvas, &main, 0, 0, cols, rows, ink, &[], &tf, None);
        let span = ink_span(&canvas, 0..canvas.height(), 0..canvas.width(), ground)
            .unwrap_or_else(|| panic!("{run:?} drew no ink at all"));
        assert_eq!(
            span, measured_at_2x,
            "amiga-arthur-text.png measures {measured_at_2x} device px of ink for {run:?}; \
             the composite drew {span}",
        );
    }
}

/// **The wrap is by PIXEL.** A row carries as many characters as fit its width in
/// the face's own advances, not a column count.
///
/// The capture's evidence is that full prose lines end within one word's width of
/// the same margin while carrying DIFFERENT character counts — 71 on the first line
/// and fewer on the fourth. Both halves are pinned: the rows differ in length, and
/// each one is full to the pixel, which is the property a column wrap cannot have.
///
/// Note the face is WIDER per character than the cell, not narrower: it averages
/// 5.21 face px against the cell's 4, so a pixel wrap fits FEWER characters on a
/// line than the 73 columns the story was told it has. That is the same fact as the
/// machine showing 20 text rows where lanthorn gave the story 25.
#[test]
fn raster_prose_wraps_by_pixel_and_fills_the_line() {
    let Some((_session, machine)) = boot() else { return };
    let tf = machine.text_face();
    let mut state = raster_state(&machine);
    // One long paragraph off the capture's own screen.
    let para = "You are shivering in the cold night air of an English churchyard, unsure \
                of how you came to be here, when suddenly a great stone appears before you \
                with a sword thrust deep into it and words engraved upon its face.";
    state.push_transcript_kind(para, app::state::TranscriptKind::Story);
    let (cols, rows) = (73u16, 8u16);
    let box_px = cols as u32 * u32::from(tf.cell().w());
    let (main, _) = app::render::screen::build_main_text(&state, cols, rows);
    let full: Vec<&String> = main.lines.iter().filter(|l| !l.is_empty()).collect();
    assert!(full.len() >= 3, "non-vacuity: the paragraph must wrap several times, got {full:?}");
    for line in &full {
        assert!(
            tf.run_px(line) <= box_px,
            "{line:?} is {} px in a {box_px} px box — the wrap must be by pixel",
            tf.run_px(line),
        );
    }
    // Different character counts on rows that are all full — which is exactly what
    // no column count can produce.
    let counts: std::collections::BTreeSet<usize> =
        full[..full.len() - 1].iter().map(|l| l.chars().count()).collect();
    assert!(
        counts.len() > 1,
        "a pixel wrap gives rows of DIFFERENT lengths; a column wrap gives one length: {counts:?}",
    );
    // The rows are full in PIXELS and short in COLUMNS, which is the whole of it:
    // the face averages 5.21 face px against the cell's 4, so a line fills before
    // its 73 columns are spent. A column wrap would run every row out to within a
    // word of the column budget instead.
    let cell = tf.cell();
    assert!(
        full[..full.len() - 1]
            .iter()
            .all(|l| cell.run_px(l) + 4 * u32::from(cell.w()) < box_px),
        "every full row must stop well short of its COLUMN budget — the pen ran out \
         first: {:?}",
        full.iter().map(|l| (l.chars().count(), tf.run_px(l))).collect::<Vec<_>>(),
    );
    // …and every one of them is full to the pixel: the next row's first word could
    // not have been added without crossing the margin.
    for pair in full.windows(2) {
        let next_word = pair[1].split(' ').next().unwrap_or("");
        let would_be = tf.run_px(pair[0]) + tf.advance(' ') + tf.run_px(next_word);
        assert!(
            would_be > box_px,
            "{:?} stopped at {} px of {box_px} with {next_word:?} ({} px) still to fit — \
             that is a break by column, not by pixel",
            pair[0],
            tf.run_px(pair[0]),
            tf.run_px(next_word),
        );
    }
}

// ── 3. the grid: one run per character, one pen per line ─────────────────────

/// Arthur's score bar steps by the face's own advances, not by the engine's cell.
///
/// The bar reaches the renderer as ONE RUN PER CHARACTER at multiples of the
/// declared cell — that is `zvm` recording where its own cursor stopped, and it is
/// right. Re-joining the runs the engine's pen laid down contiguously is what lets a
/// per-glyph pen see the line at all; without it every letter is stamped at its
/// engine column and drawn at its own narrower width, which opens a gap before each
/// one. `amiga-arthur-church.png` shows the machine's own `Church`, and its glyph
/// origins step exactly the face's advances.
#[test]
fn the_score_bar_advances_by_the_faces_own_table() {
    let Some((session, state)) = in_the_churchyard() else { return };
    let tf = state.v6_text.clone();
    assert!(tf.proportional(), "non-vacuity: the pen under test is the face's");

    // Non-vacuity on the FRAME's shape: a one-row grid across the story columns,
    // published as per-character pixel runs, is what this case is about.
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let bar = items
        .iter()
        .find_map(|it| match &it.node {
            WinNode::Grid(g) if g.rows == 1 && !g.px_texts.is_empty() && it.y_px == 200 => {
                Some((it, g))
            }
            _ => None,
        })
        .expect("the churchyard frame carries a one-row status grid at native y=200");
    assert!(
        bar.1.px_texts.len() > 20,
        "non-vacuity: the bar is published one run per character, got {}",
        bar.1.px_texts.len(),
    );
    let printed: String = {
        let mut runs: Vec<_> = bar.1.px_texts.iter().collect();
        runs.sort_by_key(|t| t.x);
        runs.iter().map(|t| t.text.as_str()).collect()
    };
    assert!(
        printed.contains("Churchyard"),
        "non-vacuity: the bar names the room, got {printed:?}",
    );

    let canvas = raster(&session, &state);
    // The bar is reverse video, so its ground is the block and the glyphs are dark.
    let ground = *canvas.get_pixel(u32::from(bar.0.x_px) + 2, 210);
    let runs = glyph_runs(&canvas, 200..220, u32::from(bar.0.x_px)..u32::from(bar.0.x_px) + 120, ground);
    assert!(runs.len() >= 10, "the room name draws ten glyphs, got {runs:?}");

    // `Churchyard`, glyph by glyph: the step from one origin to the next is that
    // glyph's advance, doubled onto the unit screen.
    let expected: Vec<u32> = "Churchyar".chars().map(|c| tf.advance(c)).collect();
    let measured: Vec<u32> = runs.windows(2).take(expected.len()).map(|w| w[1].0 - w[0].0).collect();
    assert_eq!(
        measured, expected,
        "the bar must step by the face's advances (C h u r c h y a r), not by the \
         8-px cell the engine placed the runs at",
    );
    // And the numbers `amiga-arthur-church.png` measures for `Church` — 12, 10, 10,
    // 10, 10 device px between glyph origins — reached independently of the table.
    assert_eq!(
        &measured[..5],
        &[12, 10, 10, 10, 10],
        "amiga-arthur-church.png measures these five steps on the machine's own bar",
    );
}

// ── 4. bold is WIDER, because the Amiga's is ─────────────────────────────────

/// `tf_BoldSmear` is read, and a bold run advances by it.
///
/// The Amiga emboldens by smearing a glyph right and moving the pen the same amount
/// so the extra column has somewhere to live. Arthur's `char.data` states a smear
/// of one pixel. Synthesising the smear without widening — which is what `bitfont`
/// did — eats the inter-character gap, and at a 3-to-8 px proportional advance
/// there is no gap to spare, so bold words run together.
#[test]
fn a_bold_run_advances_by_the_faces_own_smear() {
    let Some((_session, machine)) = boot() else { return };
    let face = machine.faces.body().expect("char.data");
    assert_eq!(face.bold_smear, 1, "Arthur's char.data states tf_BoldSmear = 1");
    let tf = machine.text_face();
    // §8.7.1 bit 2 is bold.
    const BOLD: u8 = 2;
    for ch in ['Y', 'o', 'u', ' ', 'a', 'r', 'e'] {
        assert_eq!(
            tf.advance_styled(ch, BOLD),
            tf.advance(ch) + 2,
            "{ch:?}: a bold glyph advances one FACE pixel further, which is two native",
        );
    }
    let plain = "You are carrying:";
    assert_eq!(
        tf.run_px_styled(plain, BOLD),
        tf.run_px(plain) + 2 * plain.chars().count() as u32,
        "…and a whole bold run is that much wider — the tracking \
         machine-screenshots/amiga-arthur-inventory.png shows on its headings",
    );
}

/// **The wrap has to measure the style it is going to DRAW in** (SQ-1050).
///
/// The raster prose wrap (`render::wrap_cache::raster_wrap_extend`) measured every
/// row with `TextFace::run_px`, which is `run_px_styled(s, 0)` — the ROMAN pen —
/// while `v6_layout::draw_story_text` steps each glyph by `advance_styled(ch,
/// row_styles[col])`. Two different questions about one row, and on the one press
/// with both a proportional face and a bold smear the answers differ by the smear.
///
/// The user's report is the line Arthur prints after the soldiers leave, reached
/// through their `bold-overflow` state and reproduced here as the string alone so
/// the wrap is the only variable. Measured on this floppy's own `char.data`:
///
/// | measure | width | vs the 584 px window |
/// |---|---|---|
/// | roman | 564 px | fits — so the wrap put all 62 characters on one row |
/// | bold | 688 px | **104 px past the right edge**, thirteen characters' worth |
///
/// `zvm`'s own pen has measured with the style since SQ-1009 and breaks the same
/// line inside the window, so this is the renderer disagreeing with the engine
/// about a line they both lay out.
///
/// FALSIFY by putting `tf.run_px(&cur) + tf.advance(' ') + tf.run_px(word)` back in
/// `raster_wrap_extend`: the line comes back as ONE row measuring 688 px drawn,
/// which is the overflow as reported.
///
/// Colour-independent by construction — every number here is an advance — so it is
/// not one of the areas that pins both `honor_game_colours` modes.
#[test]
fn a_bold_prose_line_wraps_at_the_width_it_will_be_drawn_at() {
    let Some((_session, machine)) = boot() else { return };
    let tf = machine.text_face();
    const BOLD: u8 = 2;
    const LINE: &str =
        "[You have earned five experience points and two quest points.]";

    // Non-vacuity, both halves: the string fits the window when measured ROMAN —
    // so nothing but the style can make it wrap — and does NOT fit when measured
    // as it will be drawn, so the defect is reachable at all.
    assert!(
        tf.run_px(LINE) <= PROSE_WINDOW_PX,
        "roman, this line fits on one row ({} px in {PROSE_WINDOW_PX}) — that is what made          the wrap keep it there",
        tf.run_px(LINE),
    );
    assert!(
        tf.run_px_styled(LINE, BOLD) > PROSE_WINDOW_PX,
        "and bold it does not ({} px) — otherwise this case proves nothing",
        tf.run_px_styled(LINE, BOLD),
    );

    let mut state = raster_state(&machine);
    state.transcript = vec![LINE.to_string()];
    state.transcript_runs = vec![vec![app::state::StyleRun {
        start: 0,
        end: LINE.chars().count(),
        bits: BOLD,
        fg: 0,
        bg: 0,
        link: 0,
        glk_style: 0,
    }]];
    state.transcript_images = vec![None];

    let cols = (PROSE_WINDOW_PX / u32::from(tf.cell().w())) as u16;
    let (main, _) = app::render::screen::build_main_text(&state, cols, 12);
    let rows: Vec<(&String, u8)> = main
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.is_empty())
        .map(|(r, l)| (l, main.styles.get(r).and_then(|v| v.first()).copied().unwrap_or(0)))
        .collect();
    assert!(!rows.is_empty(), "the line reached the wrap");
    assert!(
        rows.iter().all(|(_, st)| st & BOLD != 0),
        "the emphasis survives the wrap — otherwise the widths below are measured roman          and the case tests nothing: {rows:?}"
    );
    for (row, style) in &rows {
        assert!(
            tf.run_px_styled(row, *style) <= PROSE_WINDOW_PX,
            "a wrapped row is drawn inside its window: {} px of {PROSE_WINDOW_PX} for {row:?}",
            tf.run_px_styled(row, *style),
        );
    }
    // …and it is the WRAP doing it, not a truncation: put back together, the rows
    // are the line the game printed.
    let rejoined = rows.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(" ");
    assert_eq!(rejoined, LINE, "the rows are the whole line, wrapped — nothing was dropped");
}

// ── 5. what the machine did that a DECLARED width cannot ─────────────────────

/// The room description as Arthur prints it, transcribed from the runs the F5
/// screen publishes. One string, so the wrap under test is the only variable.
const DESCRIPTION: &str = "You are standing in the bright moonlight of a mid-winter's night \
in a deserted English churchyard. At the foot of the church steps is a large stone with a \
jewelled sword protruding from it. The church entrance lies to the east, and just west of you \
is a large gravestone. A stone wall encircles the churchyard, but there is an ironwork gate in \
the wall to your south.";

/// The six line spans `machine-screenshots/amiga-arthur-F5.png` measures for it, in
/// device px of ink — rows 39, 59, 79, 99, 119 and 139, all starting at x=71.
///
/// The capture is 2x the machine's 320x200 frame and so is our native space, so a
/// SPAN in one is a span in the other. (An absolute x is not: the capture carries
/// the overscan border, and the offset is **43** — the description's text origin
/// reads 71 against our native 28, and the score bar's left edge agrees.)
const F5_SPANS: [u32; 6] = [568, 576, 556, 568, 566, 382];

/// Arthur's description window is 584 native px wide, which is the 640 screen less
/// the 28 px of decorated flank on either side.
const PROSE_WINDOW_PX: u32 = 584;

/// First inked column to last, from the face's own advance table.
fn ink_px(font: &blorb::bitmap_font::BitmapFont, s: &str) -> u32 {
    let (mut pen, mut first, mut last) = (0u32, None, 0u32);
    for c in s.bytes() {
        let Some(g) = font.glyph(c) else { continue };
        for r in &g.rows {
            for b in 0..8u32 {
                if r & (0x80 >> b) != 0 {
                    let v = pen + b;
                    first = Some(first.map_or(v, |f: u32| f.min(v)));
                    last = last.max(v);
                }
            }
        }
        pen += u32::from(g.width);
    }
    match first {
        Some(f) => (last - f + 1) * 2,
        None => 0,
    }
}

fn pen_px(font: &blorb::bitmap_font::BitmapFont, s: &str) -> u32 {
    s.bytes().filter_map(|c| font.glyph(c)).map(|g| u32::from(g.width) * 2).sum()
}

/// Greedy word wrap, which is what Arthur's own formatter does — verified against
/// the engine: at a declared width of 8 the game wraps the description to 73
/// columns and this model reproduces its six lines exactly.
fn wrap_cols(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > cols {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The same wrap, measured in the face's own advances instead of in columns.
fn wrap_px(font: &blorb::bitmap_font::BitmapFont, text: &str, px: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        if !cur.is_empty() && pen_px(font, &cur) + pen_px(font, " ") + pen_px(font, word) > px {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// **The Amiga MEASURED its proportional text; it did not declare an average.**
///
/// This is the finding, and it is why SQ-1009 still declares the machine's 8 for the
/// WIDTH while taking the height from the face. It was reached by trying to recover
/// a declared width and failing, which is worth keeping as a case because the
/// failure is the result.
///
/// # What was measured
///
/// Arthur's own formatter word-wraps, using the width the interpreter declares and
/// the window's own size: told 8, it wraps the description to 73 columns; told 9, to
/// 64; told 10, to 58. Each of those was driven through the real story and read back
/// out of the published runs, and `wrap_cols` reproduces every one of them exactly.
///
/// So a declared width `W` produces `floor(584 / W)` columns, and the question "what
/// did the Amiga declare" has a testable answer: the `W` whose column count wraps
/// the description where `amiga-arthur-F5.png` wraps it.
///
/// **There is no such integer.** The capture needs 65 columns; `floor(584/W)` steps
/// 73, 64, 58, 53 across W = 8, 9, 10, 11 and never lands on 65. What DOES reproduce
/// all six spans is a wrap measured in the face's own advances against the window's
/// own width — and it does so across a 13-pixel-wide band straddling 584, which is
/// the signature of a pixel rule rather than a coincidence of one text.
///
/// The tell is in the line lengths: 64, 65, 62, 65 and 63 characters, whose pen
/// widths are 570, 578, 558, 570 and 568 px. Different character counts, the same
/// pixel width, every line full to within 3% of the window. No column count does
/// that; only a measure does.
///
/// # What it means for the fix
///
/// Reproducing the machine's layout needs the ENGINE to measure text with the
/// host's face — the game asks how wide its text is and lays out from the answer.
/// That is a much larger change than a declared constant, and it is not this
/// quest's. Recorded here so the next person does not spend the afternoon hunting a
/// number that is not there.
#[test]
fn no_declared_width_reproduces_the_machines_wrap_but_a_measured_one_does() {
    let Some((_session, machine)) = boot() else { return };
    let font = machine.faces.body().expect("char.data");

    // (a) No integer declared width lands on the capture.
    for w in 6u32..=16 {
        let cols = (PROSE_WINDOW_PX / w) as usize;
        let spans: Vec<u32> = wrap_cols(DESCRIPTION, cols).iter().map(|l| ink_px(font, l)).collect();
        assert_ne!(
            spans.as_slice(),
            F5_SPANS.as_slice(),
            "a declared width of {w} native px ({cols} columns) reproduces amiga-arthur-F5.png — \
             if this ever fires, the Amiga DID declare an average and SQ-1009 should set it",
        );
    }

    // (b) A measured wrap does, and over a band rather than at a point.
    for px in [578u32, 580, 582, PROSE_WINDOW_PX, 586, 590] {
        let spans: Vec<u32> = wrap_px(font, DESCRIPTION, px).iter().map(|l| ink_px(font, l)).collect();
        assert_eq!(
            spans.as_slice(),
            F5_SPANS.as_slice(),
            "a wrap measured in the face's own advances at {px} px must reproduce \
             amiga-arthur-F5.png's six lines",
        );
    }

    // (c) …and the reason: every line is full to the PIXEL while carrying a
    // different number of characters.
    let lines = wrap_px(font, DESCRIPTION, PROSE_WINDOW_PX);
    let counts: std::collections::BTreeSet<usize> =
        lines[..lines.len() - 1].iter().map(|l| l.chars().count()).collect();
    assert!(counts.len() > 1, "the machine's lines differ in length: {counts:?}");
    for l in &lines[..lines.len() - 1] {
        let pen = pen_px(font, l);
        assert!(
            pen > PROSE_WINDOW_PX * 95 / 100 && pen <= PROSE_WINDOW_PX,
            "{l:?} is {pen} px of a {PROSE_WINDOW_PX} px line — a measured wrap fills it",
        );
    }
}

/// **Arthur's two prose windows wrap the same text to the same points.**
///
/// `amiga-arthur-F5.png` shows the room description in the UPPER description window
/// and `amiga-arthur-inventory.png` shows it in the LOWER prose window under the
/// score bar, one line further on. Line for line, at the same x origin, the spans
/// are the same — so the wrap is a fact about the WIDTH the two windows share (584
/// native px each) and not about either window's own furniture. That is what makes
/// [`no_declared_width_reproduces_the_machines_wrap_but_a_measured_one_does`]'s
/// single window width the right thing to have measured against.
#[test]
fn both_of_arthurs_prose_windows_wrap_to_the_same_width() {
    let Some((session, state)) = in_the_churchyard() else { return };
    // Measured off the captures: the inventory frame's lower window carries F5's
    // upper window from its second line on, plus the line that follows it.
    const F5_UPPER: [u32; 7] = [568, 576, 556, 568, 566, 382, 198];
    const INVENTORY_LOWER: [u32; 6] = [576, 556, 568, 566, 382, 198];
    assert_eq!(
        &F5_UPPER[1..],
        &INVENTORY_LOWER[..],
        "the same description, wrapped identically in two different windows",
    );
    // And in OUR model the two windows really are the same width, which is the
    // fact that makes that possible. Guarded against the frame, not assumed.
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let prose: Vec<u16> = items
        .iter()
        .filter(|it| {
            u32::from(it.w_px) == PROSE_WINDOW_PX
                && (matches!(&it.node, WinNode::Grid(g) if g.rows > 1)
                    || matches!(&it.node, WinNode::Buffer(b) if b.primary))
        })
        .map(|it| it.y_px)
        .collect();
    assert!(
        prose.len() >= 2,
        "both prose windows are {PROSE_WINDOW_PX} px wide — found {} at {prose:?}",
        prose.len(),
    );
    let _ = state;
}

/// **The score bar's right-hand field lands where the machine put it, because the
/// GAME measures its own text and the engine now measures it with the machine's
/// pen** (SQ-1009).
///
/// # How the game places it
///
/// An Infocom Version 6 game right-aligns by MEASURING: it prints the field to
/// output stream 3, deselects the stream, and reads back header `$30` — "the total
/// width of printing, in units" (ZMSD §7.1.2.1) — then `set_cursor`s at
/// `x_size − $30`. So the field's position is entirely a function of the width the
/// interpreter reports, and nothing in the render path contributes to it. That was
/// checked before it was assumed: 72 of the bar's runs step by the engine's own
/// pen and exactly one origin does not, and that discontinuity is the game's
/// `set_cursor`.
///
/// # What was wrong, and what fixed it
///
/// `StreamState::pop_stream3` measured the buffer at the DECLARED cell —
/// `25 × 8 = 200` units for ` St Anne's Day, Compline ` — so the game placed the
/// field at window-relative 384 and our proportional glyphs then ran 30 px off the
/// end of a bar that stops at 612. The machine measured what it was going to DRAW:
/// 222 units, window-relative 362.
///
/// `machine-screenshots/amiga-arthur-church.png` is the falsifier. Its score bar
/// spans native 29..612 — exactly the `/dump-windows` win1 — and its date field's
/// ink runs 389..602, ten pixels clear of the bar's right edge. Restore the
/// `cell.w` measurement and the field returns to 417 and overruns.
#[test]
fn the_score_bars_right_field_lands_where_the_machine_put_it() {
    let Some((session, state)) = in_the_churchyard() else { return };
    assert!(state.v6_text.proportional(), "non-vacuity: the pen under test is the face's");
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let (bar, g) = items
        .iter()
        .find_map(|it| match &it.node {
            WinNode::Grid(g) if g.rows == 1 && !g.px_texts.is_empty() && it.y_px == 200 => Some((it, g)),
            _ => None,
        })
        .expect("the churchyard frame carries a one-row status grid at native y=200");
    // Non-vacuity on the frame: the bar the capture measures, at the width it has
    // there — 584 px starting at native 28 (the capture's 29, one-based).
    assert_eq!(
        (u32::from(bar.x_px), u32::from(bar.w_px)),
        (28, PROSE_WINDOW_PX),
        "amiga-arthur-church.png's bar is 584 px wide and starts at the left flank",
    );
    let printed: String = {
        let mut runs: Vec<_> = g.px_texts.iter().collect();
        runs.sort_by_key(|t| t.x);
        runs.iter().map(|t| t.text.as_str()).collect()
    };
    assert!(printed.contains("Compline"), "non-vacuity: the bar carries the date, got {printed:?}");

    // Exactly one origin the engine's own pen did not produce — the `set_cursor`.
    // Asked in the PEN's units, which is what the cursor now advances by.
    let mut runs: Vec<&app::engine::PxText> = g.px_texts.iter().collect();
    runs.sort_by_key(|t| t.x);
    let breaks = runs
        .windows(2)
        .filter(|w| {
            u32::from(w[0].x) + state.v6_text.run_px_styled(&w[0].text, w[0].style)
                != u32::from(w[1].x)
        })
        .count();
    assert!(
        breaks >= 1,
        "non-vacuity: the date field is PLACED by the game's own set_cursor, not flowed",
    );

    let canvas = raster(&session, &state);
    // The bar is reverse video, so its ground is the block and the glyphs are dark.
    let ground = *canvas.get_pixel(u32::from(bar.x_px) + 2, 210);
    // Right of native 200 there is nothing on the bar but the date field. Stop
    // short of the bar's last column, which carries the frame's own flank.
    let ink: Vec<u32> = (200..610u32)
        .filter(|&x| (200..220u32).any(|y| *canvas.get_pixel(x, y) != ground))
        .collect();
    assert_eq!(
        (ink.first().copied(), ink.last().copied()),
        (Some(CHURCH_DATE_INK.0), Some(CHURCH_DATE_INK.1)),
        "amiga-arthur-church.png puts the date's ink at native {CHURCH_DATE_INK:?},          ten pixels inside a bar that ends at 611",
    );
}

/// The date field's ink in `machine-screenshots/amiga-arthur-church.png`, in
/// zero-based native pixels — `St Anne's Day, Compline`, 214 px of it.
const CHURCH_DATE_INK: (u32, u32) = (389, 602);

// ── 6. an isolated reverse-video run is not a status bar ─────────────────────
// ── 6b. the ENGINE wraps where the machine wrapped ───────────────────────────

/// The runs a prose window published, re-joined into one string per native row.
fn f5_rows(node: &WinNode) -> Vec<String> {
    let WinNode::Grid(g) = node else { return Vec::new() };
    let mut v: Vec<&app::engine::PxText> = g.px_texts.iter().collect();
    v.sort_by_key(|t| (t.y, t.x));
    let mut rows: Vec<(u16, String)> = Vec::new();
    for t in v {
        match rows.last_mut() {
            Some(r) if r.0 == t.y => r.1.push_str(&t.text),
            _ => rows.push((t.y, t.text.clone())),
        }
    }
    rows.into_iter().map(|(_, s)| s).collect()
}

/// **The GAME wraps its description where the machine wrapped it** (SQ-1009).
///
/// This is the second half of the same defect the score bar is the first half of,
/// and it has the same one cause: `zvm` measured text by the DECLARED cell while the
/// pen drew the face's real advances. `exec.rs`'s v6 paint path took the line width
/// as `w.grid.cols` — the window over the declared width, `584 / 8 = 73` — and broke
/// there. Seventy-three characters at the real pen is about 759 px in a 584 px
/// window, so every line was wrapped too late and its tail was cut off.
///
/// The line now breaks at the window's own PIXEL width, measured with the same
/// table the renderer draws with, and the six spans fall out exactly:
/// [`F5_SPANS`] are `machine-screenshots/amiga-arthur-F5.png`'s own, transcribed
/// off the capture at rows 39/59/79/99/119/139.
///
/// Falsified by restoring the column break: the first line comes back 73 characters
/// long, 759 px wide, and no span matches.
#[test]
fn the_engine_wraps_the_description_where_the_machine_wraps_it() {
    let Some((mut session, state)) = in_the_churchyard() else { return };
    assert!(state.v6_text.proportional(), "non-vacuity: the pen under test is the face's");
    let _ = session.submit_char(137); // F5 — the room description
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let win = items
        .iter()
        .find(|it| it.y_px == 0 && u32::from(it.w_px) == PROSE_WINDOW_PX)
        .expect("F5 opens a full-width description window at the top");
    let rows = f5_rows(&win.node);
    assert!(rows.len() >= 6, "non-vacuity: F5 prints six lines of description, got {rows:?}");
    // The whole description, unbroken — the wrap is the only variable.
    let joined = rows[..6].join(" ");
    assert_eq!(joined, DESCRIPTION, "the six lines are the description and nothing else");

    let canvas = raster(&session, &state);
    let ground = *canvas.get_pixel(600, 190);
    let line_h = state.v6_text.line_px();
    let spans: Vec<u32> = (0..6)
        .map(|n| {
            let band = n * line_h..(n + 1) * line_h;
            ink_span(&canvas, band, 0..canvas.width(), ground).expect("an inked line")
        })
        .collect();
    assert_eq!(
        spans.as_slice(),
        F5_SPANS.as_slice(),
        "amiga-arthur-F5.png measures these six line spans; a column wrap gives none of them",
    );
    assert_eq!(CAPTURE_IS_NATIVE_SCALE, 1, "a span in the capture is a span in native px");
}

/// **The grid and the pixel runs are ONE layout** (SQ-1009).
///
/// A v6 window carries two representations of one text: the raster backend draws
/// pixel-positioned runs at the face's advances, and a cell backend places the same
/// characters on the story's grid. They are two units of one print, and they have to
/// agree character for character — a run's grid cell is the address hybrid places it
/// by, so a cell the pixel measure disagrees with draws a hole.
///
/// This first shipped as two independent wrap passes, one per unit, on the theory
/// that hybrid could fill the window's 73 DECLARED columns while raster kept the
/// machine's 584-pixel breaks. It cannot, and the reason is structural rather than a
/// slip: each pass assumes it is the only one breaking, so its indices are stale the
/// moment the other breaks first, and the two swallow DIFFERENT blanks at a soft
/// break, so their line-sets are not even the same character sequence. On this frame
/// that put `in a`, `is a large` and `church entrance lies` on the next row while
/// still tagged with the previous row's cell, at columns 68, 64 and 55 running off a
/// 73-column window, and overwrote `churchyard`'s `d` with the `.` after it.
///
/// So there is one pass, one break list, and a break moves both pens. Hybrid wraps
/// where the Amiga wrapped and its lines are honestly shorter than the window.
///
/// Falsified by restoring the second wrap: `grow` stops matching the row the run's
/// own `y` falls in, and the overlap check below fails on the same three words.
#[test]
fn the_hybrid_grid_and_the_pixel_runs_are_one_layout() {
    let Some((mut session, state)) = in_the_churchyard() else { return };
    assert!(state.v6_text.proportional(), "non-vacuity: the pen under test is the face's");
    let cell = state.v6_text.cell();

    // The score bar first, because it is the case that needs the grid to keep its
    // OWN pen: Arthur prints it one character per call, so a column DERIVED from
    // the pixel cursor steps 1.3 per letter and the row grows holes.
    {
        let model = Engine::screen(&session);
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
        let bar = items
            .iter()
            .find_map(|it| match &it.node {
                WinNode::Grid(g) if g.rows == 1 && !g.px_texts.is_empty() && it.y_px == 200 => Some(g),
                _ => None,
            })
            .expect("the churchyard frame carries a one-row status grid at native y=200");
        let row: String = (1..=bar.cols).map(|c| bar.cell(1, c).ch).collect();
        assert!(
            row.contains("Churchyard") && row.contains("St Anne's Day, Compline"),
            "the bar's grid row carries both fields unbroken, got {row:?}",
        );
    }

    let _ = session.submit_char(137); // F5
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let win = items
        .iter()
        .find(|it| it.y_px == 0 && u32::from(it.w_px) == PROSE_WINDOW_PX)
        .expect("F5 opens a full-width description window at the top");
    let WinNode::Grid(g) = &win.node else { panic!("a v6 prose window is a Grid") };
    let cols = PROSE_WINDOW_PX / u32::from(cell.w());

    // 1. Every run's grid ROW is the row its own pixel y falls in. This is the
    //    invariant two passes cannot hold, and it is the one that put a word on the
    //    wrong line.
    let mut runs: Vec<&app::engine::PxText> =
        g.px_texts.iter().filter(|t| !t.text.trim().is_empty()).collect();
    assert!(runs.len() >= 8, "non-vacuity: F5's description arrives as several runs, got {runs:?}");
    runs.sort_by_key(|t| (t.y, t.x));
    for t in &runs {
        assert_eq!(
            t.grow,
            cell.row_of(t.y),
            "run {:?} at y={} is tagged row {} but its pixel lands on row {}",
            t.text,
            t.y,
            t.grow,
            cell.row_of(t.y),
        );
    }

    // 2. No two runs on a row claim the same column — a dropped blank on one side
    //    only shows up as an overlap, which is how the `.` came to sit on the `d`.
    for pair in runs.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.grow != b.grow {
            continue;
        }
        let a_end = u32::from(a.gcol) + a.text.chars().count() as u32;
        assert!(
            u32::from(b.gcol) >= a_end,
            "{:?} ends at column {a_end} and {:?} starts at {} on row {}",
            a.text,
            b.text,
            b.gcol,
            a.grow,
        );
    }

    // 3. Both representations are the same lines, and they fit the window.
    let px_rows = f5_rows(&win.node);
    assert!(
        px_rows.join(" ").starts_with(DESCRIPTION),
        "the runs carry the description: {px_rows:?}",
    );
    for l in &px_rows {
        assert!(
            l.chars().count() <= cols as usize,
            "{l:?} is {} characters in a {cols}-column window",
            l.chars().count(),
        );
    }
    let grid_rows: Vec<String> = (1..=g.rows)
        .map(|r| (1..=g.cols).map(|c| g.cell(r, c).ch).collect::<String>().trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if !grid_rows.is_empty() {
        assert_eq!(
            grid_rows[..px_rows.len().min(grid_rows.len())],
            px_rows[..px_rows.len().min(grid_rows.len())],
            "the grid and the runs are one layout, not two",
        );
    }
}

// ── 7. the window bounds the pen ─────────────────────────────────────────────

/// **A proportional run stops at its window's right edge.**
///
/// ZMSD §8.8's window property 7 is a right margin and `zvm` lays its own prose out
/// against `x_size − right_margin`, but nothing under `render/` had ever consulted
/// either: the renderer drew rightward from a run's origin with no bound at all. At
/// a fixed cell that was invisible, because our text was NARROWER than the machine's
/// proportional face and a run always finished inside the box the game reserved for
/// it. The pen removed the slack and the omission surfaced — it exposed this rather
/// than causing it.
///
/// The engine now wraps at the pen too, so the game no longer HANDS the renderer a
/// line that overruns — see
/// [`the_engine_wraps_the_description_where_the_machine_wraps_it`]. The clamp stays
/// because a game may still `set_cursor` a run near an edge, and because the
/// non-vacuity below is what makes reaching the edge possible at all: every full
/// line of F5's description now fills its window to within 5%.
///
/// (Arthur sets both margins to zero on every window, measured; the bound is the
/// window's own width. The property is honoured anyway, because a renderer that
/// ignores it is how this happened.)
#[test]
fn a_proportional_run_stops_at_its_windows_right_edge() {
    let Some((mut session, state)) = in_the_churchyard() else { return };
    let _ = session.submit_char(137); // F5 — the long description, wrapped by the
                                      // game at the declared width and therefore
                                      // wider than the window once drawn.
    let model = Engine::screen(&session);
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame is Layered") };
    let win = items
        .iter()
        .find(|it| it.y_px == 0 && u32::from(it.w_px) == PROSE_WINDOW_PX)
        .expect("F5 opens a full-width description window at the top");
    assert_eq!((win.left_margin, win.right_margin), (0, 0), "Arthur declares no margins");
    // Non-vacuity: the lines really do reach for the edge. Every full line is
    // within 5% of the window's width and none exceeds it — which is the property
    // a column wrap cannot have, and the reason a clamp is worth testing.
    let rows = f5_rows(&win.node);
    assert!(rows.len() >= 6, "non-vacuity: F5 prints six lines of description, got {rows:?}");
    for r in &rows[..5] {
        let pen = state.v6_text.run_px(r);
        assert!(
            pen > PROSE_WINDOW_PX * 95 / 100 && pen <= PROSE_WINDOW_PX,
            "{r:?} is {pen} px of a {PROSE_WINDOW_PX} px line — a measured wrap fills it",
        );
    }

    let native = v6::native_extent(items, &state.v6_text);
    let layout = v6::classify_windows(items, state.v6_text.cell());
    let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // Nothing is drawn right of the window, which is where the decorated flank is.
    let right = u32::from(win.x_px) + u32::from(win.w_px);
    let page = *canvas.get_pixel(right + 8, 190);
    for y in 0..200u32 {
        for x in right..canvas.width() {
            assert_eq!(
                *canvas.get_pixel(x, y),
                page,
                "a glyph reached native ({x}, {y}), outside the {PROSE_WINDOW_PX} px \
                 window that bounds it",
            );
        }
    }
}




// ── 8. what HYBRID draws ─────────────────────────────────────────────────────

/// Render the frame `session` is standing on through the hybrid path, and read
/// every terminal row back as a string.
///
/// `raster()` above measures the pixel composite; this measures the CELL buffer,
/// which is the shipped default and the surface the score bar is actually read on.
fn hybrid_rows(session: &GameSession, state: &mut app::state::AppState) -> Vec<String> {
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let model = Engine::screen(session);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    state.config.v6_render = app::config::V6RenderMode::Raster;
    (0..area.height)
        .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

/// **Hybrid places a proportional run by its GRID CELL, not by dividing its pixel
/// origin** (SQ-1009).
///
/// `render/screen.rs`'s painted-screen path used to reconstruct a terminal column
/// as `cell.col_of(t.x)` — the run's native pixel origin over the DECLARED cell
/// width of 8. At a pen advancing ~10.4 px per character that quotient climbs 1.3
/// per run, so columns are skipped and the drift compounds along the line:
/// `Churchyard` came out as `Ch  urc   hy  ard`, and the wider the pane the worse
/// it read. It is the same defect the engine's own grid had, one layer out.
///
/// The engine already maintains a dense character grid beside the pixel runs, so
/// the column is not something the renderer has to derive: every run now carries
/// the screen grid cell its first character was written at
/// ([`zvm::screen::V6Text::grow`]/[`col`](zvm::screen::V6Text::gcol)), and hybrid
/// places by that.
///
/// Asserted on the CELLS the renderer stamped, because
/// `the_hybrid_grid_still_wraps_at_the_windows_own_columns` asserts on the engine's
/// grid and passed for a whole round while this was broken on screen.
#[test]
fn hybrid_draws_the_score_bar_in_consecutive_cells() {
    let Some((session, mut state)) = in_the_churchyard() else { return };
    assert!(state.v6_text.proportional(), "non-vacuity: the pen under test is the face's");
    let rows = hybrid_rows(&session, &mut state);
    assert!(
        rows.iter().any(|r| r.contains("Churchyard")),
        "the room name must read as one word in the cell buffer; got {:?}",
        rows.iter().filter(|r| r.contains("hurch") || r.contains("urc")).collect::<Vec<_>>(),
    );
    assert!(
        rows.iter().any(|r| r.contains("St Anne's Day, Compline")),
        "…and so must the date field; got {:?}",
        rows.iter().filter(|r| r.contains("Anne")).collect::<Vec<_>>(),
    );
}

/// The same, for the F5 description — a window that WRAPS, where the pixel line
/// and the grid line break in different places.
#[test]
fn hybrid_draws_the_description_in_consecutive_cells() {
    let Some((mut session, mut state)) = in_the_churchyard() else { return };
    let _ = session.submit_char(137); // F5
    let rows = hybrid_rows(&session, &mut state);
    // Six phrases from across the description, each of which spans several runs.
    for want in [
        "You are standing in the bright moonlight",
        "deserted English churchyard",
        "jewelled sword protruding from it",
        "ironwork gate in the wall to your south",
    ] {
        assert!(
            rows.iter().any(|r| r.contains(want)),
            "{want:?} must read contiguously in the cell buffer; got {rows:?}",
        );
    }
}

// ── 8. reversed spaces are FURNITURE, not a bar ──────────────────────────────

/// **A row of reversed SPACES is a rule, not a band — and a rule stops where the
/// window that owns it stops** (SQ-1035).
///
/// A reverse-video space is a solid block, which is how these games draw a line.
/// Arthur's F3 inventory paints two column dividers as one reversed space per row,
/// and its status bar one window below is ALSO entirely reversed — but that row
/// carries `Churchyard`. `machine-screenshots/amiga-arthur-inventory.png` shows what
/// the machine makes of the difference: a bare page with two thin rules down it, and
/// a status bar filled edge to edge with dark letters on white. The rules stop at the
/// bar and do not continue into the story window below it.
///
/// lanthorn got both halves wrong in hybrid. Treating "every run reversed" as a band
/// flooded seven rows of the page white, and drawing the divider columns down the
/// whole chrome strip — rather than between the rows the game painted them on — ran
/// them through the bar and four rows past it.
///
/// The engine's own runs for this frame, which is what the assertions below are
/// measured against: window 2 (584x200 at 29,1) paints `x=213` and `x=413` as a
/// reversed `" "` on every row from y=21 to y=181, and window 1 (584x20 at 29,201)
/// paints the bar. Sixteen turns and a `look`, then F3.
#[test]
fn reversed_spaces_rule_the_inventory_page_instead_of_flooding_it() {
    let Some((mut session, mut state)) = in_the_churchyard() else { return };
    let _ = session.submit_char(135); // F3 — the inventory screen

    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    let area = ratatui::layout::Rect::new(0, 0, 90, 34);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let model = Engine::screen(&session);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let reversed = |y: u16| -> Vec<u16> {
        (0..area.width)
            .filter(|&x| buf[(x, y)].modifier.contains(ratatui::style::Modifier::REVERSED))
            .collect()
    };
    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string()
    };

    // Non-vacuity: the frame under test really is the inventory over a status bar.
    let bar = (0..area.height)
        .find(|&y| row_text(y).contains("Churchyard"))
        .expect("the F3 frame carries the status bar");
    assert!(
        (0..bar).any(|y| row_text(y).contains("red piece of glass")),
        "non-vacuity: the inventory listing is above the bar",
    );

    // The BAR is a band: reversed edge to edge.
    assert_eq!(
        reversed(bar).len(),
        area.width as usize,
        "the status row carries text and floods the full width, as the capture shows",
    );

    // Every row ABOVE it is bare page with exactly the two rules on it — never a
    // flooded row, which is what the user reported.
    let rules = reversed(bar.saturating_sub(1));
    assert_eq!(rules.len(), 2, "the page carries two column rules, got {rules:?}");
    for y in 1..bar {
        assert_eq!(
            reversed(y),
            rules,
            "row {y} of the inventory page must be two rules, not a flooded band",
        );
    }

    // …and the RULE COLUMNS are gone below the bar: they belong to window 2, which
    // ends there. Asserted on those columns rather than on "nothing is reversed",
    // because the input caret one row from the bottom is legitimately a reversed cell.
    for y in bar + 1..area.height {
        let still = reversed(y);
        for c in &rules {
            assert!(
                !still.contains(c),
                "column {c} is a rule from window 2, but is still drawn on row {y}, \
                 below the bar the window ends at",
            );
        }
    }
}

/// **…and the same in RASTER, where the fill that floods is a different routine
/// entirely** (SQ-1026).
///
/// The cell backend and the pixel backend reached this defect by different roads and
/// only the cell one is fixed by `row_is_reverse_bar`. `v6_layout::fill_reverse_row_gaps`
/// fills the GAPS around a pure reverse-video row, and the two frames it has to serve
/// have nearly identical runs:
///
/// * Journey's IbmPc menu paints ONE reversed space, at x=233, on every row. Its
///   frame's side borders are not runs at all — they ARE these gaps, reaching the
///   screen edges while the over-art test suppresses the middle because the picture
///   is there. `journey_amiga_flank_border_is_a_stroke_not_a_filled_block` pins it.
/// * Arthur's F3 inventory paints TWO reversed spaces, at x=213 and x=413 — its column
///   rules — on a page with no picture at all, so nothing was suppressed and the same
///   code flooded seven rows white.
///
/// The runs do not separate them; the ARTWORK does. A row carrying TEXT is a band and
/// fills regardless (Arthur's own status row, all-reversed and holding `Churchyard`,
/// which `machine-screenshots/amiga-arthur-inventory.png` shows filled edge to edge).
/// A textless row fills only where part of it sits over a picture. A textless,
/// pictureless row is furniture on a bare page.
///
/// Falsified by dropping the artwork clause: rows 3..9 come back at 576 of 580 pixels.
#[test]
fn the_raster_inventory_page_keeps_its_rules_and_loses_the_flood() {
    let Some((mut session, state)) = in_the_churchyard() else { return };
    let _ = session.submit_char(135); // F3 — the inventory screen
    let canvas = raster(&session, &state);
    let row_h = u32::from(state.v6_text.cell().h());
    let ground = *canvas.get_pixel(canvas.width() - 3, canvas.height() - 3);

    // Ink across the middle of text row `r`, ignoring the outer margins.
    let ink = |r: u32| -> Vec<u32> {
        let y = r * row_h + row_h / 2;
        (30..canvas.width() - 30).filter(|&x| *canvas.get_pixel(x, y) != ground).collect()
    };
    let usable = canvas.width() - 60;

    // The BAR: all-reversed AND carrying text, so it floods. Found by being the
    // widest inked row, and guarded as a real band rather than assumed.
    let bar = (0..canvas.height() / row_h)
        .max_by_key(|&r| ink(r).len())
        .expect("the frame has rows");
    assert!(
        ink(bar).len() as u32 > usable * 3 / 4,
        "non-vacuity: the status row is a band filled edge to edge, got {} of {usable}",
        ink(bar).len(),
    );

    // The PAGE above it: the two column rules and nothing else. Twelve pixels — two
    // six-wide rules — against the 576 a flooded row gives.
    let rules = ink(bar - 1);
    assert!(
        (2..=24).contains(&(rules.len() as u32)),
        "the inventory page carries two thin rules, not a band: {} px of {usable}",
        rules.len(),
    );
    let (first, last) = (rules[0], rules[rules.len() - 1]);
    assert!(
        first > 150 && last < 500,
        "the rules are interior columns (the game paints them at x=213 and x=413), got {first}..{last}",
    );
    for r in 3..bar {
        assert_eq!(ink(r), rules, "row {r} of the inventory page must be the same two rules");
    }
}

/// SQ-1156: the declared cell is a HOST DECLARATION, and an in-game `@restart`
/// must re-state it.
///
/// The reported symptom is this suite's own subject seen from the other end. The
/// floppy's release face declares an 8x20 line (case 1 at the top of this file),
/// and Arthur lays his score bar out on the line he is TOLD — property 13, which
/// he reads for exactly that. `@restart` builds a fresh `ScreenState`, whose
/// windows take their `font_size` from the header, and reloads the header to the
/// story's pristine image; the metric itself is a field on `Machine` and survives.
/// So after a restart the machine said 8x20 and the windows said 8x16, and Arthur
/// laid the bar out on a 16-row line the host was not drawing on.
///
/// What the player sees is the bar coming back as PIXELS. Hybrid draws a strip
/// with glyphs only while the game's own runs explain it (CLAUDE.md: "never
/// rasterise what the game printed as a character"), and runs placed off the cell
/// boundary no longer do — so the classifier fell through to the ring and the
/// score bar returned as half-blocks. The measured frame at 80x25, before the fix:
/// row 13 went from `     Churchyard          St Anne's Day, Compline` to
/// `   ▄▄▄▀▀▀▀▀▀▀▀▀▄▀▄▄▄▄▄…`, and window 0 grew from 584x180 to 584x192.
///
/// Turn count: fifteen intro taps and a `look` — [`in_the_churchyard`]'s route —
/// then `restart`, `y`, and the same route again.
///
/// FALSIFY by removing the `set_v6_text` call from `zvm`'s `Machine::restart`:
/// the post-restart frame carries no `Churchyard` cell text at all.
#[test]
fn the_score_bar_comes_back_as_cells_after_an_in_game_restart() {
    let Some((mut session, machine)) = boot() else { return };
    let mut state = raster_state(&machine);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());

    // Non-vacuity: this case can only see the defect on a press whose declared
    // cell is NOT the 8x16 the pristine header carries. The floppy's face is what
    // makes it 8x20, and if that ever stops being true the assertions below would
    // pass while saying nothing.
    assert_eq!(
        machine.text_face().cell(),
        zvm::screen::V6Cell::new(8, 20),
        "{FIXTURE}: the release face declares a 20-row line, which is what a restart \
         has to restate",
    );

    let walk = |s: &mut GameSession| {
        for _ in 0..15 {
            let r = match s.pending_input() {
                app::session::InputKind::Line => s.submit(""),
                app::session::InputKind::Char => s.submit_char(13),
                app::session::InputKind::Event => s.submit(""),
            };
            if r.transcript.to_lowercase().contains("y or n") {
                let _ = s.submit_char(b'n');
            }
        }
        let _ = s.submit("look");
    };
    // The bar as terminal CELLS: the row carrying both fields as real buffer text.
    let bar_row = |s: &GameSession| -> Option<(u16, String)> {
        let model = Engine::screen(s);
        let area = ratatui::layout::Rect::new(0, 0, 80, 25);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        (0..area.height)
            .map(|y| {
                let t: String = (0..area.width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                    .collect();
                (y, t)
            })
            .find(|(_, t)| t.contains("Churchyard") && t.contains("St Anne's Day"))
    };

    walk(&mut session);
    let before = bar_row(&session).unwrap_or_else(|| {
        panic!("{FIXTURE}: the launch draws the score bar as terminal cells")
    });

    let r = session.submit("restart");
    assert!(
        r.transcript.to_lowercase().contains("y or n"),
        "{FIXTURE}: `restart` reaches Arthur's own confirmation: {:?}",
        r.transcript
    );
    let _ = session.submit_char(b'y');
    assert!(!session.quit, "{FIXTURE}: the restart re-booted rather than quitting");
    walk(&mut session);

    let after = bar_row(&session).unwrap_or_else(|| {
        panic!(
            "{FIXTURE}: the score bar must come back as terminal cells after an in-game \
             @restart, not as ring art"
        )
    });
    assert_eq!(
        before, after,
        "{FIXTURE}: the rebooted frame is the launch frame — same row, same text — \
         because a restart re-runs the same story on the same declared cell",
    );
}
