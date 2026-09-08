//! Shogun's Apple IIgs press floats its room illustration IN the prose and lets
//! it SCROLL with the text, exactly as its Amiga press already does — SQ-0888.
//!
//! THE REPORT, and its correction. The five-volume ProDOS set drew the opening
//! Bridge illustration as a three-row sliver across the top of the pane while the
//! prose ran full width underneath it. A first fix pinned the art in the ring's
//! flank at the game's own coordinates; the user then checked a rendition of the
//! original and reported that the ship **moves up as new text arrives**, with the
//! prose wrapping around its bottom. Pinned art cannot do that, so the fix was the
//! wrong shape however much better it looked on turn one — which is why the cases
//! below drive several turns and assert on the art MOVING, not on where it sits.
//!
//! THE TWO PRESSES STATE ONE LAYOUT TWO WAYS, and the difference is spelling:
//!
//! | press                     | draw                        | window 0 then         |
//! |---------------------------|-----------------------------|-----------------------|
//! | Amiga r295/s890321        | `pic 7 → win 0` at (229,1)  | (47,33) 548x368 L2 R328 |
//! | Apple IIgs r311/s890510   | `pic 7 → win 6` at (249,1)  | (1,65) 560x320 L0 R320  |
//!
//! Both reserve ~320 px of window 0 for the same ship on the same scene. The Amiga
//! calls `set_margins` on the window it drew into, so the engine attaches it to the
//! event and `is_win0_inline_float` reads ZMSD §15's margin picture; the Apple
//! draws from WINDOW 6 — a graphics window at (1,33) 560x352 that contains window 0
//! outright — and then calls `set_margins(0, 320, win 0)` while window 6 is still
//! current, so `PictureEvent::margin_after` is `None` and the float has to read the
//! margin **in force** instead. That is the whole of the fix (`ceded_margin_float_x`).
//!
//! THE AMIGA IS THE ACCEPTANCE CRITERION, not a description: it renders this scene
//! correctly today, so the cases that matter assert the two presses agree in SHAPE.
//! Measured at a 100x40 pane under kitty, on the emitted stream:
//!
//! * Amiga — 27 placements, one per text row, 49 cols x 1 row at cols 41..89.
//! * Apple — 27 placements, one per text row, 55 cols x 1 row at cols 43..97.
//!
//! …against the merged-but-wrong pinned placement, which by the third turn had
//! **no ship on the frame at all** while the prose it belongs to was still on it.
//!
//! `stories/` is gitignored, so every case skips vacuously.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::inline_image::ImageAlign;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TranscriptElem};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The five-volume 5.25-inch ProDOS press, named by the exact build it carries.
const APPLE: (&str, u16, &str) = ("shogun_s1.dsk", 311, "890510");
/// The Amiga press: the reference rendition of this scene, and the one whose shape
/// the Apple's must match.
const AMIGA: (&str, u16, &str) = ("James Clavell's Shogun.adf", 295, "890321");
/// The IBM/Blorb press, which spells it the Amiga's way too (SQ-0888's lane found
/// both carrying `right_margin` 328 on this frame).
const IBM: (&str, u16, &str) = ("shogun-r322-s890706.z6", 322, "890706");

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile, the artwork, the native screen
/// and the art scale all come from the medium. The Apple press needs every one of
/// them: read through the Blorb-shaped path instead it comes up 640x400 at scale
/// (2,2) rather than its own 560x384 at (4,2), and none of the coordinates below
/// are its own any more.
///
/// Answers with the session and the Bridge turn's transcript elements — the ordered
/// text/image stream the app lays out, which is where a float lives.
fn boot(press: (&str, u16, &str)) -> Option<(GameSession, Vec<TranscriptElem>)> {
    let (file, release, serial) = press;
    let path = stories_dir().join(file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file}: this medium carries a DIFFERENT build than the table says"
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
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
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // Two keypresses: past the title splash, then START on the credits menu. The
    // opening Bridge scene is the frame this file measures — the one turn on which
    // the game holds a margin open beside its illustration.
    let mut elems = Vec::new();
    for _ in 0..2 {
        let r = match s.pending_input() {
            InputKind::Char => s.submit_char(13),
            _ => s.submit(""),
        };
        elems = r.transcript_elems;
    }
    Some((s, elems))
}

/// Play one turn and answer with its ordered elements.
fn turn(s: &mut GameSession, cmd: &str) -> Vec<TranscriptElem> {
    play(s, cmd).transcript_elems
}

/// Play one turn and answer with the whole result.
fn play(s: &mut GameSession, cmd: &str) -> app::session::TurnResult {
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(cmd),
    }
}

/// Feed a turn into the transcript exactly as the app's run-loop does: the ordered
/// element stream when the turn carries one (an inline picture, a screen-clear
/// boundary), the flat text otherwise. Getting this wrong is silent — the element
/// list is EMPTY on an ordinary turn, so an ingest that only reads elements builds
/// a transcript that never grows, and every scroll assertion passes vacuously.
fn ingest(state: &mut app::state::AppState, r: &app::session::TurnResult) {
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(
            &r.transcript,
            app::state::TranscriptKind::Story,
            &r.transcript_runs,
        );
    } else {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
    }
}

/// The one float on a turn: `(width, height, align)`. The `ImageSource` that used
/// to ride along was dropped with the enum in SQ-0895 — every float reaching the
/// transcript is story content now, so the field distinguished nothing.
fn only_float(elems: &[TranscriptElem]) -> Option<(u32, u32, ImageAlign)> {
    let mut it = elems.iter().filter_map(|e| match e {
        TranscriptElem::Image(im) => Some(im),
        _ => None,
    });
    let im = it.next()?;
    assert!(it.next().is_none(), "this scene anchors exactly one picture");
    Some((im.pixels.width(), im.pixels.height(), im.align))
}

/// Window 0's box and margins on the frame now on screen.
fn story_box(s: &GameSession) -> Option<app::engine::PositionedWindow> {
    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { return None };
    app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.cloned()
}

/// Every opaque pixel the CHROME graphics windows put inside window 0's box.
///
/// The float's own definition, from the other side: art that flows with the prose
/// is not on a window canvas, so nothing chrome paints may stand where the prose
/// is going to be laid out.
fn chrome_pixels_inside_story(s: &GameSession) -> Option<usize> {
    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { return None };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let story = layout.story?;
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
    let (x0, y0) = (u32::from(story.x_px), u32::from(story.y_px));
    let (x1, y1) = ((x0 + u32::from(story.w_px)).min(gfx.width()), (y0 + u32::from(story.h_px)).min(gfx.height()));
    Some(
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| gfx.get_pixel(x, y)[3] >= 128)
            .count(),
    )
}

/// Pane sizes every rendering case runs at: the reference 100x40 terminal minus
/// lanthorn's own chrome, a cramped one, and a large one.
const PANES: &[(u16, u16)] = &[(98, 38), (78, 28), (157, 59)];

/// An `AppState` set up the way the app runs this frame: hybrid, terminal-default
/// theme, and a HALF-BLOCK picker — which is what lets a case assert on the pane's
/// own CELLS, because the ring's bands and the transcript's float strips both land
/// in them instead of in an out-of-band image protocol.
#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn app_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

fn render(s: &GameSession, state: &app::state::AppState, pane: (u16, u16)) -> Buffer {
    let model = s.screen();
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    buf
}

/// Window 0's box in PANE CELLS — the region the transcript lays out in, mapped
/// through the letterbox's uniform scale exactly as the hybrid split does.
///
/// Everything outside it is the ring's chrome, and the ring is where the Amiga
/// press keeps its two ornamental side pillars: art that spans the whole pane
/// height and never moves. Measuring the float without excluding them measures the
/// pillars instead, and reads "nothing scrolled" on the very press that does.
fn viewport(s: &GameSession, pane: (u16, u16)) -> (u16, u16, u16, u16) {
    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { return (0, pane.0, 0, pane.1) };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let story = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.cloned().expect("window 0");
    let u = ((f32::from(pane.0) * 8.0) / native.0 as f32)
        .min((f32::from(pane.1) * 18.0) / native.1 as f32);
    let cell = |px: u32, size: f32, up: bool| -> u16 {
        let v = px as f32 * u / size;
        (if up { v.ceil() } else { v.floor() } as u16).min(if size == 8.0 { pane.0 } else { pane.1 })
    };
    (
        cell(u32::from(story.x_px), 8.0, true),
        cell(u32::from(story.x_px) + u32::from(story.w_px), 8.0, false),
        cell(u32::from(story.y_px), 18.0, true),
        cell(u32::from(story.y_px) + u32::from(story.h_px), 18.0, false),
    )
}

/// Is this cell showing the game's own ink rather than a grey? The story page, the
/// theme backdrop and the prose are all neutral on this frame in every press, so a
/// channel spread of any size is artwork. (The same test `v6_shogun_menu_ground`
/// uses, for the same reason.)
fn is_art(buf: &Buffer, x: u16, y: u16) -> bool {
    [buf[(x, y)].bg, buf[(x, y)].fg].iter().any(|c| {
        let ratatui::style::Color::Rgb(r, g, b) = *c else { return false };
        r.max(g).max(b) - r.min(g).min(b) >= 24
    })
}

/// The first and last pane row inside the STORY VIEWPORT carrying the game's art.
fn art_span(buf: &Buffer, vp: (u16, u16, u16, u16)) -> Option<(u16, u16)> {
    let rows: Vec<u16> =
        (vp.2..vp.3).filter(|&y| (vp.0..vp.1).any(|x| is_art(buf, x, y))).collect();
    Some((*rows.first()?, *rows.last()?))
}

/// Is there prose written BELOW `row` in the RIGHT half of the story viewport —
/// i.e. in the columns the picture had been holding, past its bottom edge?
fn text_below(buf: &Buffer, row: u16, vp: (u16, u16, u16, u16)) -> bool {
    let mid = vp.0 + (vp.1 - vp.0) / 2;
    ((row + 1)..vp.3)
        .any(|y| (mid..vp.1).any(|x| buf[(x, y)].symbol().chars().any(|c| c.is_alphanumeric())))
}

// ── 1. The premise: this frame really is the one the quest measured ─────────

/// The Apple press states its layout in window properties, and draws from window 6.
///
/// Everything below rests on these facts, so they are asserted rather than assumed:
/// if a later build of the loader resolves this medium differently, this case
/// should say so instead of the ones after it quietly measuring some other screen.
#[test]
fn the_apple_press_reserves_a_margin_on_window_0_and_paints_it_from_window_6() {
    let Some((s, _)) = boot(APPLE) else { return };
    let story = story_box(&s).expect("the Bridge scene is a layered v6 frame");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 64, 560, 320),
        "window 0's box on the Bridge"
    );
    assert_eq!(
        (story.left_margin, story.right_margin),
        (0, 320),
        "the game reserves the right 320 px of window 0 for its illustration"
    );
    // And window 6 — the graphics window it drew the ship from — CONTAINS window 0,
    // which is the first of the three things `ceded_margin_float_x` asks for.
    let model = s.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("layered") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let contains = layout.chrome.iter().any(|c| {
        c.x_px <= story.x_px
            && c.y_px <= story.y_px
            && c.x_px + c.w_px >= story.x_px + story.w_px
            && c.y_px + c.h_px >= story.y_px + story.h_px
    });
    assert!(
        contains,
        "the ship comes from a graphics window that contains window 0 outright \
         (win 6 at (1,33) 560x352) — that containment is what tells this apart from \
         a neighbouring window that merely overlaps the prose"
    );
}

// ── 2. The reported behaviour: a float, of the Amiga's own shape ────────────

/// The illustration is an inline MARGIN FLOAT on both presses — the same kind of
/// thing, at each press's own size.
///
/// This is the whole correction. A float is anchored in the text stream, so it
/// scrolls with the prose and the prose wraps past its bottom; art on a window
/// canvas is pinned to the pane and can do neither. The Apple used to produce no
/// float at all, because `is_win0_inline_float` rejected everything with
/// `ev.window != 0`.
#[test]
fn both_presses_anchor_the_ship_as_a_right_margin_float_in_the_prose() {
    let mut ran = 0;
    for (press, size) in [(APPLE, (312, 348)), (AMIGA, (320, 370))] {
        let Some((_, elems)) = boot(press) else { continue };
        ran += 1;
        let got = only_float(&elems).unwrap_or_else(|| {
            panic!(
                "{} [release {}]: the Bridge turn anchors NO picture in its prose. The ship is \
                 ZMSD §15's margin picture — the game reserves a column of window 0 with \
                 `set_margins` and paints the illustration into it — and it has to reach the \
                 text stream to scroll with the text. The Apple press states this by drawing \
                 from window 6 and setting the margin on window 0; reading the window number \
                 as a difference in kind is what dropped it.",
                press.0, press.1
            )
        });
        assert_eq!(
            got,
            (size.0, size.1, ImageAlign::MarginRight),
            "{} [release {}]: the ship floats at the RIGHT margin as story content",
            press.0,
            press.1
        );
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun press present — every case skipped");
}

/// …and nothing chrome paints inside window 0's box any more.
///
/// The other half of the same statement, and the reason the first fix's machinery
/// could be REMOVED rather than kept alongside this one. That fix narrowed window
/// 0's box to the text column and handed the ceded column to the ring, because
/// window 6's canvas held the ship. It does not any more — the ship went into the
/// text stream — so the column is empty, the narrowing has nothing to fire on, and
/// two mechanisms are not left fighting over the same columns.
#[test]
fn the_ship_leaves_the_chrome_canvas_entirely() {
    let Some((mut s, _)) = boot(APPLE) else { return };
    for t in 0..4 {
        let inside = chrome_pixels_inside_story(&s).expect("layered v6 frame");
        assert_eq!(
            inside, 0,
            "turn {t}: {inside} chrome pixels stand inside window 0's box. The ship is a float \
             now; a graphics window still painting where the prose lays out would mean the art \
             exists twice and the two copies would part company the moment the text scrolls."
        );
        turn(&mut s, "look");
    }
}

// ── 3. The defining property: it MOVES with the prose ───────────────────────

/// The ship scrolls up as text accumulates, and the prose runs past its bottom.
///
/// THE CASE THE MERGED FIX WOULD FAIL, and the reason it exists: a placement
/// assertion about turn one passes against art pinned in the ring just as happily
/// as against a float. Only the change ACROSS turns tells them apart — and pinned
/// art fails it twice over, because by the third turn of this scene the ring's
/// flank had no ship on it at all while the prose it belongs to was still on screen
/// (the game drops the margin, the flank goes with it, and the picture simply
/// vanishes).
///
/// Measured on the rendered pane, which is the only place the answer is visible.
#[test]
fn the_ship_scrolls_up_with_the_prose_and_the_text_runs_past_its_bottom() {
    let mut ran = 0;
    for press in [APPLE, AMIGA] {
        let mut rings = 0;
        for (&pane, honor) in PANES.iter().flat_map(|p| [(p, true), (p, false)]) {
            // A fresh session per pane: the drive below plays six turns, and reusing
            // one session would start the next pane six turns further into the game.
            let Some((mut s, elems)) = boot(press) else { continue };
            ran += 1;
            let mut state = app_state(honor);
            app::state::apply_transcript_elems(&mut state, &elems);
            let vp = viewport(&s, pane);
            let mut spans = Vec::new();
            let mut ring = true;
            let mut buf = render(&s, &state, pane);
            for t in 0..7 {
                if state.v6_path_log.borrow().last().map(|(l, _)| l.as_str()) != Some("hybrid-ring")
                {
                    // A pane too small to read prose in falls back to the whole-frame
                    // composite, where there is no text layer to scroll and nothing
                    // here to measure. `PANES` deliberately keeps one such pane, so
                    // this is a skip and not a pane the case pretends to cover.
                    ring = false;
                    break;
                }
                if let Some(sp) = art_span(&buf, vp) {
                    spans.push(sp);
                } else if t == 0 {
                    panic!(
                        "{} [release {}] {pane:?} honor={honor}: no artwork anywhere in the story viewport \
                         (rows {}..{}) on the Bridge turn",
                        press.0, press.1, vp.2, vp.3
                    );
                }
                let r = play(&mut s, "look");
                ingest(&mut state, &r);
                buf = render(&s, &state, pane);
            }
            if !ring {
                continue;
            }
            rings += 1;
            let first = spans[0];
            let last = *spans.last().expect("at least the first frame");
            assert!(
                last.1 < first.1,
                "{} [release {}] {pane:?} honor={honor}: the illustration's bottom row is {} after six turns \
                 and was {} on the first — it has not moved. The art is anchored in the TEXT, so \
                 it must ride up the pane as prose arrives beneath it; art that keeps its rows \
                 while the transcript grows is pinned to the pane, which is the shape this quest \
                 replaced. Spans by turn: {spans:?}",
                press.0,
                press.1,
                last.1,
                first.1
            );
            assert!(
                spans.windows(2).all(|w| w[1].1 <= w[0].1),
                "{} [release {}] {pane:?} honor={honor}: the illustration moved back DOWN the pane at some \
                 point ({spans:?}) — a float only ever scrolls one way",
                press.0,
                press.1
            );
            // …and the prose reclaims the picture's own columns once it is past it.
            assert!(
                text_below(&buf, last.1, vp),
                "{} [release {}] {pane:?} honor={honor}: nothing is written across the pane below the \
                 illustration's last row ({}). The wrap must reclaim the full width once the \
                 text has passed the picture's bottom edge — that is what \"the text wraps \
                 around the bottom of the ship\" means, and a picture the prose can never get \
                 past is not a float.",
                press.0,
                press.1,
                last.1
            );
        }
        assert!(
            rings > 0 || ran == 0,
            "{} [release {}]: no pane took the hybrid ring",
            press.0,
            press.1
        );
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun press present — every case skipped");
}

/// The chrome rule above window 0 stays whole on every turn, and it is ALL the
/// chrome canvas carries.
///
/// The Apple press draws picture 4 into window 6 as a 560x32 rule between the
/// status window and the prose, and the ship used to be painted into that very same
/// canvas over the top of it. The user, watching a rendition of the original, noted
/// that the art transiently covers this rule and the rule is redrawn over the art
/// on the next scroll — a hardware sequencing artifact we are deliberately NOT
/// imitating, but whose settled state (chrome above art) is the one to reach and a
/// rule PERMANENTLY lost under the picture would be ours to answer for.
///
/// A float never touches the canvas, so the rule stays whole and the canvas holds
/// nothing else: the second assertion is what makes this discriminating, because it
/// fails the moment any picture is painted onto a chrome window again.
#[test]
fn the_rule_above_the_prose_stays_whole_on_every_turn() {
    let Some((mut s, _)) = boot(APPLE) else { return };
    for t in 0..4 {
        let model = s.screen();
        let app::engine::WinNode::Layered(items) = &model.root else { panic!("layered") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("window 0");
        // The band between the status window's bottom and window 0's top.
        let band: Vec<u32> = (32..u32::from(story.y_px))
            .filter(|&y| (0..gfx.width()).any(|x| gfx.get_pixel(x, y)[3] >= 128))
            .collect();
        assert!(!band.is_empty(), "turn {t}: the rule above the prose is gone entirely");
        for &y in &band {
            let opaque = (0..gfx.width()).filter(|&x| gfx.get_pixel(x, y)[3] >= 128).count();
            assert_eq!(
                opaque,
                gfx.width() as usize,
                "turn {t}: native row {y} of the rule is {opaque} of {} px wide, not the whole \
                 width — the chrome the game draws above its prose is being clipped.",
                gfx.width()
            );
        }
        let total = (0..gfx.height())
            .flat_map(|y| (0..gfx.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| gfx.get_pixel(x, y)[3] >= 128)
            .count();
        assert_eq!(
            total,
            band.len() * gfx.width() as usize,
            "turn {t}: the chrome canvas carries {total} opaque pixels but the rule is only \
             {} of them. Something else is painting on a chrome window — the illustration is \
             the candidate, and on this scene it lands ON the rule.",
            band.len() * gfx.width() as usize
        );
        turn(&mut s, "look");
    }
}

// ── 4. …and the presses that draw into window 0 do not move ────────────────

/// The Amiga and IBM presses hold `right_margin` 328 on this frame, draw into
/// window 0, and must keep behaving exactly as they did.
///
/// They are the reference, so a regression here is worse than the original defect:
/// `ceded_margin_float_x` only ever answers for `ev.window != 0`, and this is the
/// case that keeps it that way.
#[test]
fn the_window_0_presses_are_untouched() {
    let mut ran = 0;
    for press in [AMIGA, IBM] {
        let Some((s, elems)) = boot(press) else { continue };
        ran += 1;
        let story = story_box(&s).expect("layered v6 frame");
        assert_eq!(
            story.right_margin, 328,
            "{} [release {}]: this press reserves 328 px beside its ship",
            press.0, press.1
        );
        assert_eq!(
            only_float(&elems).map(|f| f.2),
            Some(ImageAlign::MarginRight),
            "{} [release {}]: its ship is the window-0 margin float it has always been",
            press.0,
            press.1
        );
        assert_eq!(
            chrome_pixels_inside_story(&s),
            Some(0),
            "{} [release {}]: nothing chrome paints inside window 0 here either",
            press.0,
            press.1
        );
    }
    assert!(ran > 0 || !stories_dir().exists(), "no window-0 Shogun press present — every case skipped");
}
