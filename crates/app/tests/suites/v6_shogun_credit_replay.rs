//! Shogun's credits are painted ONCE — SQ-0890.
//!
//! Reported off ghostty preview captures at 100x40 under kitty, one keypress past
//! the splash: the credits appear twice. Correctly up top, where the game painted
//! them; and again at the bottom left, replayed by the transcript into the
//! four-row prose box the game has just moved window 0 down to — running straight
//! into the START / RESTORE / QUIT menu so that a credit line and a menu item
//! share a row and abut with no gap. Verbatim, at the bottom left:
//!
//! ```text
//!   Amiga r295/s890321 :  Copyright (c) 1988 by InfocomQUIT the game
//!   Blorb r322/s890706 :  Copyright (c) 1988 by I QUIT the game
//! ```
//!
//! WHERE THE SECOND RENDITION CAME FROM. The game prints its nine credit lines
//! while window 0 is the whole 640x400 screen, then moves and resizes window 0
//! into a 548x64 box at the bottom. ZMSD §15 is explicit that this "does not
//! change the current display", so the engine FREEZES the stranded prose where it
//! was painted (SQ-0697) and the app publishes it as its own paint layer. But the
//! host went on carrying the same characters in its transcript as scrollback, and
//! the story box re-renders the transcript — so the composite drew the credits a
//! second time, into a box four rows tall that the menu is sitting in.
//!
//! WHAT THE FIX IS. Pictures had had this rule since SQ-0461: an image the canvas
//! already carried was marked `ImageSource::ContentSplash`, and the modes that
//! render the canvas skipped it rather than draw it twice. Canvas-painted TEXT had
//! no equivalent. It has one now — the engine's own retirement stamp, which says
//! exactly "these characters are paint on the screen now" — and the host stops
//! emitting the frozen head into the transcript at all.
//!
//! The picture-side twin is gone as of SQ-0895: it existed to feed the frameless
//! mode, which was the only thing that ever drew those bands, so removing the mode
//! left nothing to emit them for. The rule this suite asserts is unaffected — it
//! was always the text-side statement, and it is now the only one.
//!
//! WHY CELL-VS-RASTER PARITY DOES NOT CATCH IT, and why this suite exists beside
//! `v6_shogun_menu_ground.rs`: that suite asserts hybrid matches raster
//! cell-for-cell and passed the whole time the collision was on screen, because
//! raster carried the identical duplication. The property has to be asserted
//! directly — no row of the prose box may carry a credit, at any scroll offset.
//!
//! THREE RENDITIONS, because the medium is incidental (CLAUDE.md) and the presses
//! do not even reach the screen the same way: the Amiga and IBM presses stream the
//! credits as prose and freeze them, while the Apple IIgs press paints them as
//! positioned runs that never enter the transcript at all — which is why it was
//! reported clean, and why it belongs here as the control. `stories/` is
//! gitignored, so every case skips vacuously.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TurnResult};

struct Rendition {
    file: &'static str,
    release: u16,
    serial: &'static str,
}

const RENDITIONS: &[Rendition] = &[
    Rendition { file: "James Clavell's Shogun.adf", release: 295, serial: "890321" },
    Rendition { file: "shogun-r322-s890706.z6", release: 322, serial: "890706" },
    // The five-volume 5.25-inch ProDOS press; `app::hints` mounts the whole set
    // from whichever volume is named.
    Rendition { file: "shogun_s1.dsk", release: 311, serial: "890510" },
];

/// Phrases from the credits block, chosen to appear in every press. A row of the
/// prose box carrying any of these is the reported defect.
const CREDIT_FRAGMENTS: &[&str] =
    &["A Story of Japan", "Copyright (c) 1988 by Infocom", "SHOGUN is a trademark"];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does, one keypress past the splash — the credits,
/// the prompt and the menu — handing back the session and the turn it produced.
fn boot(r: &Rendition) -> Option<(GameSession, String, TurnResult)> {
    let path = stories_dir().join(r.file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        r.release,
        "{}: this medium carries a DIFFERENT build than the table says",
        r.file
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), r.serial, "{}: serial", r.file);
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", r.file));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let banner = s.take_transcript();
    let turn = match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    Some((s, banner, turn))
}

/// The app's own transcript after that boot — built the way `startup.rs` and
/// `turn.rs` build it, so what this measures is what a player's pane holds.
#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn app_state(banner: &str, turn: &TurnResult, honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Raster;
    state.config.honor_game_colours = honor;
    state.push_transcript(banner);
    if turn.transcript_elems.is_empty() {
        state.push_transcript_runs(
            &turn.transcript,
            app::state::TranscriptKind::Story,
            &turn.transcript_runs,
        );
    } else {
        app::state::apply_transcript_elems(&mut state, &turn.transcript_elems);
    }
    state
}

/// Every pixel-positioned run on the frame, reassembled into one line per pixel
/// row (runs sorted left to right). The Apple press prints its menu — and its
/// credits — one glyph per run through a 1px caret window, so nothing here can be
/// matched run by run.
fn canvas_rows(session: &GameSession) -> Vec<String> {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let mut by_row: std::collections::BTreeMap<u16, Vec<(u16, String)>> = Default::default();
    for it in items {
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                by_row.entry(t.y).or_default().push((t.x, t.text.clone()));
            }
        }
    }
    by_row
        .into_values()
        .map(|mut runs| {
            runs.sort_by_key(|(x, _)| *x);
            runs.into_iter().map(|(_, t)| t).collect::<String>()
        })
        .collect()
}

/// The story window's own box, in cells of the 8x16 v6 font — the prose box the
/// composite renders the transcript into (`build_v6_raster_canvas`).
fn prose_box_cells(session: &GameSession) -> (u16, u16) {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story.expect("the credits screen has a story window");
    ((story.w_px / 8).max(1), (story.h_px / 16).max(1))
}

// ── 1. The premise: the menu really does share the prose box's rows ──────────

/// Any prose the box carries lands ON the menu, so "no credit in the box" and "no
/// row carries both a credit and a menu item" are the same statement.
///
/// Stated as its own case because it is the reason case 2 is worth asserting: if
/// the menu sat clear of the story window, a duplicated credit would be untidy
/// rather than the reported garbage.
#[test]
fn the_menu_sits_inside_the_story_windows_own_box() {
    for r in RENDITIONS {
        let Some((session, _, _)) = boot(r) else { continue };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let story = layout.story.expect("the credits screen has a story window");
        let (top, bottom) = (story.y_px, story.y_px + story.h_px);

        let menu: Vec<u16> = items
            .iter()
            .filter_map(|it| match &it.node {
                WinNode::Grid(g) => Some(g.px_texts.iter()),
                _ => None,
            })
            .flatten()
            .filter(|t| t.text.chars().all(|c| c.is_ascii_uppercase() || c == ' '))
            .map(|t| t.y)
            .collect();
        assert!(!menu.is_empty(), "{}: the menu prints as runs on this screen", r.file);
        let rows = canvas_rows(&session).join("\n");
        for item in ["START", "RESTORE", "QUIT"] {
            assert!(rows.contains(item), "{}: {item} is on the frame: {rows:?}", r.file);
        }
        assert!(
            menu.iter().any(|&y| y >= top && y < bottom),
            "{}: the menu's rows ({menu:?}) fall inside the story box ({top}..{bottom})",
            r.file
        );
    }
}

// ── 2. The reported symptom, stated directly ────────────────────────────────

/// No row of the prose box carries a credit — at ANY scroll offset.
///
/// Swept rather than measured at one offset on purpose. The pager parks the view
/// where a turn's new output begins, which on the boot turn is the top of the
/// credits, and that park is what put them on the menu; sweeping asserts the
/// stronger property that they are not reachable in this box at all, which is
/// what "the canvas already has them" means.
#[test]
fn no_row_of_the_prose_box_ever_carries_a_credit() {
    for r in RENDITIONS {
        let Some((session, banner, turn)) = boot(r) else { continue };
        let (cols, rows) = prose_box_cells(&session);
        for honor in [true, false] {
            let state = app_state(&banner, &turn, honor);
            let (_, metrics) = app::render::screen::build_main_text(&state, cols, rows);
            for scroll in 0..=metrics.max_scroll.saturating_add(2) {
                let mut state = app_state(&banner, &turn, honor);
                state.transcript_scroll = scroll;
                let (main, _) = app::render::screen::build_main_text(&state, cols, rows);
                for line in &main.lines {
                    for frag in CREDIT_FRAGMENTS {
                        assert!(
                            !line.contains(frag),
                            "{} (honor={honor}, scroll={scroll}): the prose box replays a \
                             credit the canvas has already painted, on the menu's own rows: \
                             {line:?}",
                            r.file
                        );
                    }
                }
            }
        }
    }
}

// ── 3. …and nothing was lost to get there ───────────────────────────────────

/// The credits still reach the canvas, and the one line the game printed into the
/// window's NEW box still reaches the transcript.
///
/// The guard against a fix that simply deletes text: suppressing a rendition is
/// only safe while the other one exists, and the boundary must not swallow what
/// the game printed after it.
#[test]
fn the_canvas_keeps_the_credits_and_the_transcript_keeps_the_prompt() {
    for r in RENDITIONS {
        let Some((session, banner, turn)) = boot(r) else { continue };
        let rows = canvas_rows(&session).join("\n");
        for frag in CREDIT_FRAGMENTS {
            assert!(rows.contains(frag), "{}: the canvas still paints {frag:?}: {rows:?}", r.file);
        }
        let state = app_state(&banner, &turn, true);
        let transcript = state.transcript.join("\n");
        assert!(
            transcript.contains("You may choose to"),
            "{}: the line the game printed at the window's new origin survives: {transcript:?}",
            r.file
        );
        for frag in CREDIT_FRAGMENTS {
            assert!(
                !transcript.contains(frag),
                "{}: the transcript still carries {frag:?}, which is paint on the screen: \
                 {transcript:?}",
                r.file
            );
        }
    }
}

// ── the frozen layer's BOX is the pen's extent (SQ-1062) ─────────────────────

/// **A retired entry's box is measured with the PEN, not the declared cell.**
///
/// `session::v6_screen_model` synthesises a box for the frozen-prose layer, and
/// its own comment says what that box is meant to be: *"the frozen prose's own
/// extent, not the window's"*. It measured `chars * cell.w` — which is
/// `V6Cell::run_px`, documented in `zvm` as *"uniform on purpose, even for a
/// machine that painted proportionally"*. That is the right number for a box a
/// GAME reserved and the wrong one for an extent recovered from runs, and
/// `build_chrome_canvas` then turns this `w_px` into the proportional pen's CLIP
/// BOUND and floods `fill_explicit_bg_rows` to the same short width.
///
/// # Why the metric is swapped rather than a proportional press booted
///
/// The two facts have to meet — a proportional pen AND retired prose — and no
/// medium in `stories/` puts them together. Measured across the corpus: Arthur's
/// Amiga floppy is the one press with a proportional release face and it produces
/// **no** retired runs on any route driven here; Shogun's Amiga floppy produces
/// **nine** and its face is fixed; the Macintosh presses are proportional only
/// with a System disk the repo cannot carry (SQ-1036, and the trap SQ-1052's first
/// case fell into).
///
/// The model is a pure function of the windows and the metric, so this boots the
/// real frame that HAS the runs and hands it the pen. The runs keep the positions
/// the fixed pen gave them, which is exactly right: what is under test is how the
/// box is measured FROM them.
///
/// FALSIFY by restoring `(t.text.chars().count() as u16) * font_w`: `w_px` comes
/// back as the declared width and the assertion reports both numbers.
#[test]
fn a_frozen_prose_box_is_the_pens_extent_not_the_declared_one() {
    let r = &RENDITIONS[0]; // the Amiga floppy — the press that freezes its credits
    let Some((mut s, _banner, _turn)) = boot(r) else { return };

    // The premise: this frame really does carry a frozen layer.
    let retired: Vec<(String, u8)> = s
        .machine
        .screen
        .v6
        .as_ref()
        .map(|v6| {
            v6.windows.iter().flat_map(|w| w.retired.iter()).map(|t| (t.text.clone(), t.style)).collect()
        })
        .unwrap_or_default();
    assert!(!retired.is_empty(), "{}: this frame freezes its credits", r.file);

    // A pen that VARIES, so the two measures can differ at all. Synthetic for the
    // reason in the header; only its advances matter here.
    let glyph = |w: u8| blorb::bitmap_font::Glyph { width: w, rows: vec![0xFF; 16] };
    let font = blorb::bitmap_font::BitmapFont {
        width: 12,
        height: 16,
        baseline: 13,
        bold_smear: 0,
        proportional: true,
        lo: b' ',
        glyphs: (b'\x20'..=b'\x7e').map(|c| glyph(9 + (c % 4))).collect(),
    };
    let profile = InterpreterProfile::Amiga;
    let tf = app::native_font::TextFace::new(
        profile,
        app::native_font::FaceSet::release(font, profile, Some((1, 1))),
        Some((1, 1)),
    );
    assert!(tf.proportional(), "the precondition: a pen that varies");
    let cell = s.machine.v6_cell();
    let declared: u32 =
        retired.iter().map(|(t, _)| u32::from(cell.w()) * t.chars().count() as u32).max().unwrap_or(0);
    let penned: u32 = retired.iter().map(|(t, st)| tf.run_px_styled(t, *st)).max().unwrap_or(0);
    assert!(
        penned > declared,
        "non-vacuity: the pen must be WIDER than the declared cell here ({penned} vs \
         {declared}), or the two measures agree and this proves nothing",
    );

    s.machine.v6_metric = tf.metric().clone();
    let model = Engine::screen(&s);
    let WinNode::Layered(items) = &model.root else { panic!("{}: v6 Layered root", r.file) };

    // The frozen entry is the one whose runs are the retired ones: it is a Grid
    // carrying them, and the widest run in it decides the box.
    let widest_declared = declared as u16;
    let frozen = items
        .iter()
        .filter(|pw| {
            matches!(&pw.node, WinNode::Grid(g)
                if g.px_texts.iter().any(|t| retired.iter().any(|(r, _)| *r == t.text)))
        })
        .max_by_key(|pw| pw.w_px)
        .unwrap_or_else(|| panic!("{}: the frozen layer reaches the model", r.file));
    assert!(
        frozen.w_px > widest_declared,
        "{}: the frozen box is the PEN's extent — got {} px, which is the declared \
         `chars * {}` measure ({widest_declared} px) rather than the pen's ({penned} px)",
        r.file,
        frozen.w_px,
        cell.w(),
    );
}
