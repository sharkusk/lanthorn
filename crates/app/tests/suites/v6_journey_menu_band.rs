//! **The menu is the fixed-height window; the art and the story take what is left.**
//! SQ-0765, and the border it left stranded three rows up, SQ-0754.
//!
//! Every other v6 title in the corpus puts its fixed window at the TOP — Arthur's
//! status bar, Zork Zero's banner — and lets the story grow downward. Journey is the
//! inversion: its command menu lives along the BOTTOM, and it is the menu whose height
//! is a property of itself. The user stated the rule:
//!
//! > "we have 3 main windows. the artwork on left, story window on the right, and the
//! > menu system on the bottom. the menu system should be treated as fixed 'y' size
//! > (pixels->terminal rows), but dynamic 'x'. The art and story are dynamic based on
//! > the width and the remaining height of the screen available above the menu."
//!
//! The planner had it exactly backwards. The story viewport was the scale-derived
//! quantity — the story window's box through the letterbox — and the menu band was
//! simply whatever fell out below it, so the band's height wandered with the pane
//! while its content stayed a constant seven game rows. Measured off the user's own
//! dumps: 9 rows at pane height 61 / scale 1.43, 11 rows at 61 / 1.96. Three
//! consequences, all one defect:
//!
//!   * the rows the content never reached were painted by nothing, which is why the
//!     frame's own `└────┘` sat three rows above the pane's last row (SQ-0754);
//!   * an entirely empty `menu:art` upload trailed the band at some pane sizes,
//!     because a run-less row inside a full-width band classifies as art; and
//!   * at a short pane the arithmetic ran the other way and the band came out
//!     SHORTER than its content, clipping the last menu line off the screen.
//!
//! The fix is the principle: the band's height is the span of the GAME text rows the
//! menu carries — hybrid draws chrome text one game row per terminal row (SQ-0543),
//! so that span IS "pixels → terminal rows" — bottom-anchored to the pane, with the
//! story viewport taking everything above it.
//!
//! Driven on BOTH releases, because they are different builds and their menus are
//! different shapes: `Journey - The Quest Begins.adf` is release 30 / serial 890322
//! (Amiga, a line-drawing frame whose top rule and `└────┘` are menu rows 18 and 24,
//! seven rows in all), `journey-r83-s890706.z6` is release 83 / serial 890706 (IBM PC
//! by default, no box glyphs — a reverse-video header at row 19 and "Game" at row 24,
//! six rows in all). A rule that is right on one build can be wrong on the other.
//!
//! Swept across pane widths AND heights — the band's old height was a function of
//! both, so a single sample could not have seen it — and in both `honor_game_colours`
//! modes, per the project's colour-render convention.

use std::path::PathBuf;

use app::engine::{Engine, PxText, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::screen::{compose_v6_frame, RasterMetrics, V6FrameInputs};
use app::render::v6_layout::{self as v6, MainText, RasterFrame, V6TextMode};
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// A rect as `/dump-windows` records it: `(x, y, w, h)`.
type Quad = (u16, u16, u16, u16);

/// The v6 text cell is 8x16 (SQ-0479).
const FONT_H: u32 = 16;

/// Pane widths swept. 80 is the game's own column count, where the letterbox scale is
/// one native pixel per device pixel and a placement defect can hide.
const WIDTHS: [u16; 6] = [80, 96, 110, 115, 138, 150];

/// Pane heights swept. Every one of these leaves the ring vertical slack to reclaim
/// (`18·rows > 5·cols` at an 8x18 cell), which is what keeps the plan `Menu`; each case
/// asserts the plan it got, so the sweep cannot quietly drift into another regime.
const HEIGHTS: [u16; 4] = [45, 51, 61, 68];

/// The two releases, with the profile the medium picks for each. Release 30 comes off
/// the floppy and resolves to Amiga; release 83 is the bare story file and resolves to
/// the IBM PC. Both are Journey, and neither stands in for the other.
const RELEASES: [(&str, u16); 2] = [("Journey - The Quest Begins.adf", 30), ("journey-r83-s890706.z6", 83)];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a Journey release exactly as `startup.rs` does — the profile comes from the
/// medium — and drive the intro to the Praxix command menu. `None` (with a SKIP note)
/// when the gitignored fixture is absent.
fn boot(file: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let loaded = match app::hints::load_mounted_story(&path) {
        Ok((l, _)) => l,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        loaded.bytes().to_vec(),
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    // SQ-1393: the machine's own colour table. `new_with_trace` is the
    // no-machine door and presents §8.3.1's own, so a harness that boots a
    // press states the press's table here.
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..40 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(s)
}

/// A hybrid render at real kitty-ish cell metrics (8x18). `Picker::halfblocks()` reports
/// a 1x2 cell — a layout regime that reproduces no scale defect at all — so the sweep
/// runs at a plausible font cell (the SQ-0548 lesson).
#[allow(deprecated)]
fn render_pane(
    model: &app::engine::ScreenModel,
    honor: bool,
    pane: Quad,
    transcript: &str,
) -> (app::state::AppState, Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    let area = Rect::new(pane.0, pane.1, pane.2, pane.3);
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (state, area, buf)
}

/// Every paint run the game has on screen.
fn chrome_runs(model: &app::engine::ScreenModel) -> Vec<PxText> {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The menu's own runs — everything the game printed BELOW its story window — bucketed
/// by the GAME text row they sit on. This is the oracle for the whole file: the band's
/// height is how many of these rows there are, and nothing about the pane.
fn menu_rows_of(model: &app::engine::ScreenModel) -> std::collections::BTreeMap<u32, Vec<PxText>> {
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let story = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT).story.expect("story window");
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    let mut out: std::collections::BTreeMap<u32, Vec<PxText>> = Default::default();
    for t in chrome_runs(model) {
        let py = t.y.max(1) as u32 - 1;
        if py >= story_bottom {
            out.entry(py / FONT_H).or_default().push(t);
        }
    }
    out
}

/// The band's `/dump-windows` records: every `menu:*` line this frame emitted.
fn menu_records(state: &app::state::AppState) -> Vec<(String, Quad)> {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .filter(|e| e.label.starts_with("menu:"))
        .map(|e| (e.label.clone(), e.cells))
        .collect()
}

fn viewport_of(state: &app::state::AppState) -> Quad {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "viewport")
        .map(|e| e.cells)
        .expect("a hybrid ring frame records its story viewport")
}

/// One terminal row of the buffer as a string.
fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right()).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

// ── (a) SQ-0765: the menu's height is the menu's, and the story takes the rest ──

/// The band is exactly as tall as the menu's own game rows, sits flush with the pane's
/// bottom, and the story viewport runs down to meet it — at every pane size, on both
/// releases, in both colour modes.
///
/// The height being the SAME number at every pane size is the load-bearing half: before
/// the fix it was a remainder, so it moved with the letterbox scale.
///
/// FALSIFY by restoring the old `menu_top` (the first terminal row carrying a menu run
/// through the bottom-anchored menu scale) in `render/screen.rs`'s `BottomPlan::Menu`
/// arm: release 30 fails with `the menu band is 9 rows for a menu of 7 game rows` at
/// 115x61 and `11 rows` at 157x61, and release 83 with `8`/`9` for its 6.
#[test]
fn the_menu_band_is_its_own_height_bottom_anchored_and_the_story_takes_the_rest() {
    for (file, release) in RELEASES {
        let Some(mut session) = boot(file) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let rows = menu_rows_of(&model);
        assert!(!rows.is_empty(), "{file} (r{release}): Journey prints a command menu below its story window");
        let want = (rows.keys().max().unwrap() - rows.keys().min().unwrap() + 1) as u16;

        for honor in [true, false] {
            for w in WIDTHS {
                for h in HEIGHTS {
                    let (state, area, _buf) = render_pane(&model, honor, (0, 0, w, h), &transcript);
                    let plan = state.v6_ring_plan.get();
                    assert_eq!(plan, "menu", "{file} (r{release}) {w}x{h} honor={honor}: this sweep is the Menu plan");

                    let recs = menu_records(&state);
                    assert_eq!(
                        recs.len(),
                        1,
                        "{file} (r{release}) {w}x{h} honor={honor}: the menu band is ONE text strip — a second \
                         record is a run-less row classified as art, which is the empty upload SQ-0754 measured. \
                         Got {recs:?}"
                    );
                    let (label, band) = &recs[0];
                    assert!(
                        label.starts_with("menu:text"),
                        "{file} (r{release}) {w}x{h} honor={honor}: the band is text, not art. Got {label}"
                    );
                    assert_eq!(
                        band.3, want,
                        "{file} (r{release}) {w}x{h} honor={honor}: the menu band is {} rows for a menu of {want} \
                         game rows — its height is the menu's own, not the letterbox's leftover. Band {band:?}",
                        band.3
                    );
                    assert_eq!(
                        band.1 + band.3,
                        area.bottom(),
                        "{file} (r{release}) {w}x{h} honor={honor}: the menu is anchored to the pane's bottom. \
                         Band {band:?}, pane bottom {}",
                        area.bottom()
                    );
                    assert_eq!(
                        (band.0, band.2),
                        (area.x, area.width),
                        "{file} (r{release}) {w}x{h} honor={honor}: the menu's width is the pane's. Band {band:?}"
                    );

                    let vp = viewport_of(&state);
                    assert_eq!(
                        vp.1 + vp.3,
                        band.1,
                        "{file} (r{release}) {w}x{h} honor={honor}: the story takes the height remaining above the \
                         menu — its viewport {vp:?} must run down to the band at row {}",
                        band.1
                    );
                }
            }
        }
    }
}

// ── (b) SQ-0754: the frame's bottom border lands on the pane's last row ──

/// The menu's LAST game row is drawn on the pane's LAST row, and its FIRST on the band's
/// first — so nothing in the band is stranded and no row of it is painted by nothing.
///
/// On release 30 the last row is the frame's own `└────┘`, which is the quest's whole
/// symptom: it used to land three rows high at 138x68, with rows 65..67 written by
/// nothing at all. On release 83 (IBM PC, no box glyphs) it is the "Game" verb.
///
/// The characters asserted are the game's OWN — read off its runs, never chosen here —
/// per SQ-0750: we replicate what the game printed, we never invent a border glyph.
///
/// FALSIFY as above: at 138x68 release 30 fails with `the menu's last game row … is not
/// on the pane's last row 67`, the `└` and `┘` having been drawn on row 64.
#[test]
fn the_menus_last_game_row_lands_on_the_panes_last_row() {
    for (file, release) in RELEASES {
        let Some(mut session) = boot(file) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        let rows = menu_rows_of(&model);
        assert!(!rows.is_empty(), "{file} (r{release}): Journey prints a command menu below its story window");
        // The game's own ink on the menu's first and last rows: the distinct non-blank
        // texts it printed there. A reverse-video SPACE carries no glyph, so it is not
        // something a cell can be asked about.
        let ink = |row: u32| -> Vec<String> {
            let mut v: Vec<String> = rows[&row]
                .iter()
                .filter(|t| !t.text.trim().is_empty())
                .map(|t| t.text.trim().to_string())
                .collect();
            v.sort();
            v.dedup();
            v
        };
        let (first, last) = (*rows.keys().min().unwrap(), *rows.keys().max().unwrap());
        let (top_ink, bottom_ink) = (ink(first), ink(last));
        assert!(!bottom_ink.is_empty(), "{file} (r{release}): the menu's last game row {last} carries ink");

        for honor in [true, false] {
            for w in WIDTHS {
                for h in HEIGHTS {
                    let (_state, area, buf) = render_pane(&model, honor, (0, 0, w, h), &transcript);
                    let bottom = row_text(&buf, area, area.bottom() - 1);
                    for want in &bottom_ink {
                        assert!(
                            bottom.contains(want.as_str()),
                            "{file} (r{release}) {w}x{h} honor={honor}: the menu's last game row ({last}) is not on \
                             the pane's last row {} — {want:?} is missing from it. Row: {bottom:?}",
                            area.bottom() - 1
                        );
                    }
                    let top = row_text(&buf, area, area.bottom() - (last - first + 1) as u16);
                    for want in &top_ink {
                        assert!(
                            top.contains(want.as_str()),
                            "{file} (r{release}) {w}x{h} honor={honor}: the menu's first game row ({first}) is not \
                             on the band's first row {} — {want:?} is missing from it. Row: {top:?}",
                            area.bottom() - (last - first + 1) as u16
                        );
                    }
                }
            }
        }
    }
}

// ── (c) SQ-1574: a RASTER host's Menu-anchor compose path ──
//
// The TUI's own render is Hybrid, which reclaims the letterbox slack by moving
// TERMINAL CELLS around — nothing in (a)/(b) above touches `compose_v6_frame`.
// This is the OTHER path: a host whose Hybrid draws its own chrome text under
// `V6TextMode::RecordOnly` opts into `V6FrameInputs::bottom_anchor_menu` and
// gets the identical bottom-anchored band out of the RASTER composite instead
// — canvas grown, band runs re-seated, flanks filled, story between.

const HOST_INK: image::Rgba<u8> = image::Rgba([220, 220, 220, 255]);
const HOST_PAGE: image::Rgba<u8> = image::Rgba([0, 0, 0, 255]);

fn empty_prose(_cols: u16, rows: u16) -> (MainText, RasterMetrics) {
    (
        MainText { lines: Vec::new(), styles: Vec::new(), input: String::new(), cursor_col: 0, awaiting: false, floats: Vec::new() },
        RasterMetrics { total_rows: 0, viewport_rows: rows, max_scroll: 0, first_visible_row: 0 },
    )
}

/// Compose `session`'s current frame with a bare host cell (matching this
/// file's own `menu_rows_of`, `zvm::screen::V6Cell::DEFAULT`) — no `AppState`,
/// every input the host's own, exactly `v6_headless_compose.rs`'s
/// `host_compose_full` pattern.
fn menu_anchor_compose(
    session: &GameSession,
    frame: RasterFrame,
    text: V6TextMode,
    bottom_anchor_menu: bool,
) -> app::render::screen::V6Frame {
    let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let colors = app::colors::ColorScheme::terminal_default();
    let layout = v6::classify_windows(items, tf.cell());
    let inputs = V6FrameInputs {
        host_pair: (HOST_INK, HOST_PAGE),
        honor_game_colours: true,
        colors: &colors,
        face: &tf,
        paint: None,
        panel_input: None,
        input: None,
        prose: &empty_prose,
        reveal: None,
        pager_active: false,
        more_prompt_pair: (HOST_INK, HOST_PAGE),
        text,
        bottom_anchor_menu,
    };
    compose_v6_frame(&layout, frame, &inputs)
}

/// A pane tall enough to leave real surplus below Journey's story window at the
/// bare 8x16 cell this file composes at — mirrors `v6_extended_frame.rs`'s own
/// `TALL` (800x900 device px at its 8x18 kitty cell); the exact cell differs
/// but the point is the same, real surplus at a whole magnification.
const TALL_PANE_DEV: (u32, u32) = (800, 900);

/// **The compose-path acceptance case.** With `bottom_anchor_menu` on, the
/// canvas grows by the extension, the menu band's own runs land `extension`
/// native pixels lower while keeping the SAME distance from the new bottom
/// that the game put them from the screen's own, `story` fills the gap between
/// the top chrome and the relocated band, and the opened flank columns carry
/// the game's own panel colour rather than the bare story page. Declining
/// (`bottom_anchor_menu` off) is BYTE-IDENTICAL between `Raster` and
/// `Extended`, exactly as before this quest.
///
/// FALSIFY by reverting the `menu_case && !inputs.bottom_anchor_menu` bypass in
/// `screen.rs`'s `compose_v6_frame_into`: every assertion below about the
/// EXTENDED frame fails, because `anchored.canvas.height()` stays `native.1`
/// (the frame never grows) — the pre-quest behaviour this suite otherwise pins.
#[test]
fn menu_anchor_compose_bottom_anchors_the_band_and_fills_the_flanks() {
    for (file, release) in RELEASES {
        let Some(session) = boot(file) else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
        let native = v6::native_extent(items, &tf);
        let cell = tf.cell();
        let layout = v6::classify_windows(items, cell);
        let story = layout.story.expect("Journey has a story window on this frame");
        let story_bottom = story.y_px as u32 + story.h_px as u32;

        let want = RasterFrame::extended(native, TALL_PANE_DEV, cell, Some(2.0), true);
        let extension = want.extension();
        assert!(extension > 0, "{file} (r{release}): premise — this pane must actually extend");

        // Raster/Extended keep declining, byte for byte, exactly as before this
        // quest — the whole point of the flag defaulting off.
        let raster = menu_anchor_compose(&session, RasterFrame::native(native), V6TextMode::Rasterise, false);
        let extended_declined = menu_anchor_compose(&session, want, V6TextMode::Rasterise, false);
        assert_eq!(
            extended_declined.canvas.as_raw(),
            raster.canvas.as_raw(),
            "{file} (r{release}): Extended must decline Journey's menu exactly as Raster does with \
             bottom_anchor_menu off"
        );
        assert_eq!(
            extended_declined.canvas.dimensions(),
            (native.0 as u32, native.1 as u32),
            "{file} (r{release}): a declined extension must not grow the canvas"
        );

        // The Menu-anchor path.
        let anchored = menu_anchor_compose(&session, want, V6TextMode::Rasterise, true);
        assert_eq!(
            anchored.canvas.dimensions(),
            (native.0 as u32, want.canvas_h),
            "{file} (r{release}): the canvas must grow by the extension"
        );

        // The band's own runs, before and after — `RecordOnly` reports them as
        // data rather than pixels, which is the whole point of the flag (a
        // host drawing its own chrome text needs their POSITIONS).
        let before = menu_anchor_compose(&session, RasterFrame::native(native), V6TextMode::RecordOnly, false);
        let after = menu_anchor_compose(&session, want, V6TextMode::RecordOnly, true);
        let band_before: Vec<_> = before.text.iter().filter(|r| r.y >= story_bottom).collect();
        let band_after: Vec<_> = after.text.iter().filter(|r| r.y >= story_bottom + extension).collect();
        assert!(!band_before.is_empty(), "{file} (r{release}): the menu band must carry runs");
        assert_eq!(
            band_before.len(),
            band_after.len(),
            "{file} (r{release}): the moved band must carry exactly the same runs as the unmoved one — \
             before {band_before:?}, after {band_after:?}"
        );
        for (b, a) in band_before.iter().zip(band_after.iter()) {
            assert_eq!(a.text, b.text, "{file} (r{release}): a moved run's text must not change");
            assert_eq!(
                a.y,
                b.y + extension,
                "{file} (r{release}): run {:?} must land exactly `extension` ({extension}) rows lower",
                b.text
            );
            // …at the SAME distance from the frame's NEW bottom edge that the game
            // put it at from the screen's own — `bottom_anchor`'s own invariant
            // (SQ-1132), carried to the run-level mover (SQ-1574).
            assert_eq!(
                want.canvas_h - a.y,
                u32::from(native.1) - b.y,
                "{file} (r{release}): run {:?} must keep its distance from the frame's bottom edge",
                b.text
            );
        }
        // Nothing above the story's own bottom moved — the picture surround and
        // side rules the band shares a window with (SQ-1574's whole reason for
        // being) are untouched.
        let above_before: Vec<_> = before.text.iter().filter(|r| r.y < story_bottom).collect();
        let above_after: Vec<_> = after.text.iter().filter(|r| r.y < story_bottom).collect();
        assert_eq!(
            above_before.len(),
            above_after.len(),
            "{file} (r{release}): runs above the story window must be unaffected by the extension"
        );
        for (b, a) in above_before.iter().zip(above_after.iter()) {
            assert_eq!(a.y, b.y, "{file} (r{release}): run {:?} above the story window must not move", b.text);
        }

        // `story` fills the gap between the top chrome and the relocated band.
        let sbox = anchored.story.expect("Journey has a story box on this frame");
        assert_eq!(
            sbox.y + sbox.h,
            story_bottom + extension,
            "{file} (r{release}): the story box must reach exactly the relocated band's new top"
        );

        // The flank columns the extension opened carry the game's own panel
        // colour, not the bare story page — sampled well clear of any band run's
        // own glyph ink, so this cannot pass merely because a run happened to
        // cover the sampled pixel.
        let sx0 = story.x_px as u32;
        let sx1 = (story.x_px as u32 + story.w_px as u32).min(native.0 as u32);
        let flanks: [(u32, u32); 2] = [(0, sx0), (sx1, native.0 as u32)];
        let mut sampled_any = false;
        for (fx0, fx1) in flanks {
            if fx1 <= fx0 {
                continue;
            }
            // One native row above the relocated band's own top — inside the gap
            // the extension opened, outside any run's glyph box.
            let y = story_bottom + extension.saturating_sub(1);
            for x in (fx0..fx1).step_by(4) {
                let p = *anchored.canvas.get_pixel(x, y.min(anchored.canvas.height() - 1));
                assert_ne!(
                    p, HOST_PAGE,
                    "{file} (r{release}): flank pixel ({x},{y}) is the bare story page, not the game's panel"
                );
                sampled_any = true;
            }
        }
        assert!(sampled_any, "{file} (r{release}): premise — this frame must have a flank to sample");
    }
}

// ── (d) SQ-1576/SQ-1577: the raster host's flank fill, fixed ──
//
// `fill_menu_flank_extension` used to sample the CANVAS for "the nearest
// painted pixel above the gap" — empty under `RecordOnly` (a chrome run's
// background block is never actually painted there — the host paints its
// own text — SQ-1576) and, for the picture column, a smear of the picture's
// own varying bottom-row pixels into vertical stripes under EVERY mode
// (SQ-1577). Both are fixed in the same pass: the picture is recentred
// with its ground reflooded from its own panel colour (never sampled from
// the canvas), and a divider column falls back to its run's own resolved
// colour when the canvas has nothing painted to sample.

/// SQ-1576's acceptance case: the `RecordOnly` canvas equals the `Rasterise`
/// canvas pixel for pixel everywhere but inside a recorded glyph box or
/// caret — the same invariant `v6_headless_compose.rs`'s
/// `record_only_images_no_glyph_the_runs_do_not_account_for` pins for the
/// native frame, extended to the Menu-anchor's GROWN one. The gap-fill this
/// quest is about is ART, never TEXT, so it is not something a host is ever
/// asked to paint itself from the run list — it must already agree between
/// the two modes without help.
///
/// FALSIFY by reverting the `.or_else` fallback onto the run's own resolved
/// colour in `fill_menu_flank_extension`'s divider arm (`screen.rs`): every
/// pane below fails with a pixel differing outside every recorded glyph box,
/// in the gap rows below the divider's own column — exactly SQ-1576's
/// reported symptom (measured there at Journey r83, x 232..239).
#[test]
fn menu_anchor_compose_record_only_matches_rasterise_everywhere_but_the_glyphs() {
    for (file, release) in RELEASES {
        let Some(session) = boot(file) else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
        let native = v6::native_extent(items, &tf);
        let cell = tf.cell();
        let layout = v6::classify_windows(items, cell);
        let story = layout.story.expect("Journey has a story window on this frame");
        let story_bottom = story.y_px as u32 + story.h_px as u32;

        let want = RasterFrame::extended(native, TALL_PANE_DEV, cell, Some(2.0), true);
        let extension = want.extension();
        assert!(extension > 0, "{file} (r{release}): premise — this pane must actually extend");

        let full = menu_anchor_compose(&session, want, V6TextMode::RasteriseAndRecord, true);
        let bare = menu_anchor_compose(&session, want, V6TextMode::RecordOnly, true);
        assert_eq!(full.text, bare.text, "{file} (r{release}): both modes see the same text");
        assert_eq!(full.caret, bare.caret, "{file} (r{release}): both modes see the same caret");

        let cell_w = u32::from(cell.w());
        let inside = |x: u32, y: u32| {
            bare.text
                .iter()
                .any(|r| (r.y..r.y + r.h).contains(&y) && r.boxes.iter().any(|&(bx, w)| (bx..bx + w).contains(&x)))
                || bare.caret.is_some_and(|c| (c.x..c.x + cell_w).contains(&x) && (c.y..c.y + c.h).contains(&y))
        };
        let mut differing = 0usize;
        for (x, y, p) in full.canvas.enumerate_pixels() {
            if bare.canvas.get_pixel(x, y) != p {
                differing += 1;
                assert!(
                    inside(x, y),
                    "{file} (r{release}): pixel ({x},{y}) differs between RecordOnly and Rasterise outside \
                     every glyph box (story_bottom={story_bottom}, extension={extension}, so the gap is rows \
                     {story_bottom}..{})",
                    story_bottom + extension
                );
            }
        }
        assert!(differing > 0, "{file} (r{release}): RecordOnly imaged the text anyway");

        // Non-vacuity: the gap the extension opened must actually carry SOME
        // opaque ink under RecordOnly, or the loop above never exercised the
        // fill this quest is about.
        let opaque_in_gap = (story_bottom..story_bottom + extension)
            .any(|y| (0..bare.canvas.width()).any(|x| bare.canvas.get_pixel(x, y)[3] > 0));
        assert!(opaque_in_gap, "{file} (r{release}): the extension's gap carries no ink under RecordOnly — premise");
    }
}

/// SQ-1577's acceptance case: the picture is recentred in the space the
/// extension opened, with the ground around it reflooded from its own panel
/// colour — never a per-column smear of whatever pixel sat at its own bottom
/// edge, and never left pinned to the panel's own top with only the gap
/// below it filled.
///
/// FALSIFY by reverting `fill_menu_flank_extension`'s art-recentring arm
/// (`menu_flank_art`) in `screen.rs`: every pane below fails the "one flat
/// colour" assertion — the pre-quest fill samples a DIFFERENT pixel per
/// 8px block, so the probed row comes back as several distinct colours
/// instead of one.
#[test]
fn menu_anchor_compose_recentres_the_picture_instead_of_smearing_it() {
    for (file, release) in RELEASES {
        let Some(session) = boot(file) else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
        let tf = app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT);
        let native = v6::native_extent(items, &tf);
        let cell = tf.cell();
        let layout = v6::classify_windows(items, cell);
        let story = layout.story.expect("Journey has a story window on this frame");
        let story_bottom = story.y_px as u32 + story.h_px as u32;
        let sx0 = story.x_px as u32;
        let cw = u32::from(cell.w().max(1));
        // The divider columns are excluded — they carry their own ink down
        // separately (SQ-1576) and are not part of the panel's flat fill.
        // The INNER one (the last text cell before the story box) is every
        // Menu-plan release's; the Amiga press (r30) also draws an OUTER
        // one against the pane's own left edge, which the PC press (r83)
        // does not — so the first cell is skipped too, harmlessly, on a
        // release that never drew one there.
        let divider_lo = sx0.saturating_sub(cw);
        assert!(divider_lo > cw, "{file} (r{release}): premise — the left flank must be wider than two cells");

        let want = RasterFrame::extended(native, TALL_PANE_DEV, cell, Some(2.0), true);
        let extension = want.extension();
        assert!(extension > 0, "{file} (r{release}): premise — this pane must actually extend");
        let avail_h = story_bottom + extension;

        let anchored = menu_anchor_compose(&session, want, V6TextMode::Rasterise, true);

        // No vertical stripes: a row inside the gap the extension opened,
        // clear of the divider's own column, is ONE flat colour across the
        // whole picture flank.
        let probe_y = story_bottom + extension / 2;
        let mut colours: Vec<image::Rgba<u8>> =
            (cw..divider_lo).map(|x| *anchored.canvas.get_pixel(x, probe_y)).collect();
        colours.dedup();
        assert_eq!(
            colours.len(),
            1,
            "{file} (r{release}): row {probe_y} of the picture flank is {} distinct colours, not one flat \
             fill — the vertical-stripe smear SQ-1577 reported",
            colours.len()
        );
        let panel = colours[0];

        // The picture is really CENTRED, not merely top-anchored with the
        // gap filled below it: find where the flat panel colour first
        // breaks (the art's top edge) and where it last holds (the art's
        // bottom edge) over the whole available span, and check the
        // midpoint sits near avail_h / 2.
        let breaks = |y: u32| (0..divider_lo).any(|x| *anchored.canvas.get_pixel(x, y) != panel);
        let Some(new_ay0) = (0..avail_h).find(|&y| breaks(y)) else {
            panic!("{file} (r{release}): no art found anywhere in the recentred flank — premise");
        };
        let new_ay1 = (new_ay0..avail_h).rev().find(|&y| breaks(y)).unwrap_or(new_ay0);
        let mid = (new_ay0 + new_ay1) / 2;
        let tolerance = avail_h / 6;
        assert!(
            mid.abs_diff(avail_h / 2) <= tolerance,
            "{file} (r{release}): the art's centre (row {mid}) is not near the available span's own centre \
             (row {}) of {avail_h} native rows — top-anchored instead of centred",
            avail_h / 2
        );
    }
}

/// The `flank-art`/`flank-panel` records this frame's hybrid ring leaves in
/// `v6_cell_map` (SQ-0547) — the TUI's own oracle for where it puts
/// Journey's picture, read back rather than re-derived.
fn flank_art_and_panel(state: &app::state::AppState) -> Option<(Quad, Quad)> {
    let map = state.v6_cell_map.borrow();
    let art = map.iter().find(|e| e.label == "flank-art").map(|e| e.cells)?;
    let panel = map.iter().find(|e| e.label == "flank-panel").map(|e| e.cells)?;
    Some((art, panel))
}

/// SQ-1577's cross-check against the real oracle: the TUI's own hybrid ring
/// really does centre Journey's picture in the reclaimed flank panel, rather
/// than pinning it to the panel's top edge — confirming the mechanism the
/// raster host compose path above was ported from
/// ([`menu_flank_panel`](app::render::screen)), on the TUI's OWN rendering
/// of the same frame, not a description of it.
#[test]
fn the_tuis_own_hybrid_centres_the_flank_picture_in_the_panel() {
    for (file, release) in RELEASES {
        let Some(mut session) = boot(file) else { return };
        let transcript = session.take_transcript();
        let model = session.screen();
        for honor in [true, false] {
            let (state, _area, _buf) = render_pane(&model, honor, (0, 0, 150, 68), &transcript);
            let plan = state.v6_ring_plan.get();
            assert_eq!(plan, "menu", "{file} (r{release}) honor={honor}: this pane must take the Menu plan");
            let Some((art, panel)) = flank_art_and_panel(&state) else {
                panic!("{file} (r{release}) honor={honor}: no flank-art/flank-panel record — premise");
            };
            let (art_y, art_h) = (art.1, art.3);
            let (panel_y, panel_h) = (panel.1, panel.3);
            assert!(art_h > 0 && panel_h > 0, "{file} (r{release}) honor={honor}: a degenerate rect");
            let above = art_y.saturating_sub(panel_y);
            let below = (panel_y + panel_h).saturating_sub(art_y + art_h);
            assert!(
                above > 0 && below > 0,
                "{file} (r{release}) honor={honor}: the picture is flush with the panel's own top or bottom \
                 edge ({above} rows above, {below} below) instead of centred — art {art:?}, panel {panel:?}"
            );
            let tol = (panel_h / 2).max(1);
            assert!(
                above.abs_diff(below) <= tol,
                "{file} (r{release}) honor={honor}: {above} rows above the art against {below} below is not \
                 close to centred — art {art:?}, panel {panel:?}"
            );
        }
    }
}
