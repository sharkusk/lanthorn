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
