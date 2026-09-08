//! SQ-1004 — Arthur's InvisiClues screen is ONE page, not a row of islands.
//!
//! # The report
//!
//! On the Amiga floppy, every line of the hint menu carried the game's own
//! ground for exactly as many columns as it had characters and the host theme's
//! for the rest: `N for next item.` and `P for previous item.` each their own
//! block, `(Or use mouse.)`, `Return for hints.` and `Q to resume story.` three
//! more, `KING LOT`, `POINTS` and `NOTES` three short ones with the theme
//! showing to the right of each. `THE CHURCHYARD` looked right because it is a
//! REVERSED run and the reverse floods its whole row.
//!
//! # Reaching the frame — a frame is a fixture (CLAUDE.md)
//!
//! Arthur has no `HINT` verb in the sense Zork Zero does. Typing it in play
//! answers *"If only you had a crystal ball...."* on all four presses in
//! `stories/` (releases 54, 63 and 74), and the story's own text says why: *"To
//! get a hint, look into the crystal ball."* The screen is reached from the
//! DEATH prompt, whose menu offers `UNDO, RESTORE, RESTART, QUIT, or HINT` —
//! `HINT` there is Merlin lending you the ball ("Your vision blurs. It seems you
//! are standing in a darkened cave, gazing into a crystal ball"), and the menu
//! paints on the turn after that.
//!
//! So: fourteen blank turns past the intro (answering `n` to the restore
//! question), then `open gate`, `e` — which walks into the church after curfew
//! and gets you arrested — then `hint` at the death prompt, `hint` again to
//! accept, a blank line, and one keypress. [`hint_menu`] is that sequence and
//! nothing else; the shape guards below fail rather than pass quietly if a
//! release ever stops answering it.
//!
//! # What the frame is, and why RASTER is the oracle
//!
//! There is no story window on it: `classify_windows` finds only Grids, so the
//! hybrid path takes the **painted (hint/menu takeover)** arm, which stamps every
//! run as positioned terminal text and draws nothing else. All seventy-eight of
//! Arthur's runs name `fg = bg = 0` — no colour at all — so each one resolves
//! through that arm's base style, while the cells around them keep the page
//! `render_story_pane_frame`'s opening flood put down.
//!
//! Those were two different grounds. The base style was the bare
//! `upper_window` theme entry; the flood is the machine's page, because §8.3's
//! Amiga interpreter publishes a screen pair and Arthur names none of its own.
//! Measured before the fix at a 100x34 pane: `KING LOT` came out `White` on the
//! theme's `Black` for its eight columns, and `Rgb(66, 66, 66)` — the Amiga page
//! — for the ninety-two beside it.
//!
//! **Raster had it right the whole time**, and that is the strongest evidence in
//! the file: it resolves the same pair through `v6_host_pair`, whose top layer is
//! the machine pair (SQ-0740), and composes a canvas that censuses to exactly two
//! colours — 242,239 px of page against 13,761 of ink, and nothing else at all.
//! `v6_machine_page` is that function's terminal-cell counterpart, so the two
//! modes now draw one screen. [`raster_and_hybrid_draw_the_same_page`] pins the
//! agreement rather than either mode's number.
//!
//! # Falsified against a real machine
//!
//! `machine-screenshots/amiga-shogun-main.png` — Shogun release 295 / serial
//! 890321 under "Amiga Interpreter version 6.8", the same build and the same
//! interpreter `James Clavell's Shogun.adf` boots. Its text-only menu screen is
//! the grey page edge to edge: the gaps between the lines, the space to the right
//! of every line and the blank rows below them are all one ground, with white ink
//! on it, and the only patch of anything else is the deliberately reversed
//! `START the game` selection band. `machine-screenshots/c64-zork1-solidgold-hint.png`
//! is the same statement about an InvisiClues topic menu specifically — the very
//! same `N =`/`P =`/`RETURN =`/`Q = Resume story` legend, one page under the whole
//! screen. Neither machine draws a text row as an island.
//!
//! # Both `honor_game_colours` modes
//!
//! True is the report and the shipped default. False is pinned too, and pinned as
//! ABSENCE: a page the interpreter paints with is still a game colour, so with
//! colours declined the machine's page must not appear anywhere on this screen.
//! That is what keeps the fix gated.
//!
//! Skip-if-missing (gitignored media), and non-vacuous: a fixture that is present
//! but yielded no frame fails rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// The report's own medium, by exact release. A disk image is a different build,
/// not the same story on other media (CLAUDE.md).
const FIXTURE: &str = "Arthur - The Quest for Excalibur.adf";
const RELEASE: u16 = 54;
const SERIAL: &str = "890606";

/// Panes swept. 100x34 is where the islands were measured; 80x48 is the size the
/// Amiga floppy was driven at in SQ-0823, so the two frames come from real
/// sittings rather than round numbers.
const PANES: &[(u16, u16)] = &[(100, 34), (80, 48)];

/// Every string the hint menu prints, which is the frame's own signature — the
/// same five the report names, plus the section header and its topics.
const LEGEND: &[&str] = &[
    "InvisiClues (tm)",
    "N for next item.",
    "P for previous item.",
    "(Or use mouse.)",
    "Return for hints.",
    "Q to resume story.",
    "THE CHURCHYARD",
    "KING LOT",
    "POINTS",
    "NOTES",
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` boots: the profile from the MEDIUM the mount
/// returned, and the screen through `std_window → native_std_window →
/// profile.std_window` with `art_scale` alongside. Skip any step and the game
/// lays its own windows out to a screen the player never sees (CLAUDE.md).
fn boot() -> Option<GameSession> {
    let path = stories_dir().join(FIXTURE);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "{FIXTURE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), SERIAL, "{FIXTURE}: serial");
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
    .unwrap_or_else(|e| panic!("{FIXTURE}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive to the InvisiClues topic menu. See the module docs for why this route
/// and not a `hint` verb in play.
fn hint_menu() -> Option<GameSession> {
    let mut s = boot()?;
    for _ in 0..14 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        assert!(!s.quit, "{FIXTURE}: quit during the intro");
    }
    // Out through the gate, into the church after curfew: arrested, which is the
    // death prompt that offers HINT.
    for cmd in ["open gate", "e", "hint", "hint", ""] {
        let r = s.submit(cmd);
        assert!(r.fault.is_none(), "{FIXTURE}: {cmd:?} faulted: {:?}", r.fault);
    }
    assert_eq!(s.pending_input(), InputKind::Char, "{FIXTURE}: the crystal ball waits on a keypress");
    let r = s.submit_char(13);
    assert!(r.fault.is_none(), "{FIXTURE}: opening the menu faulted: {:?}", r.fault);
    Some(s)
}

fn state(honor: bool) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    // A real kitty cell size: this screen is all glyphs, but the arm it takes is
    // chosen only when there IS an image protocol.
    st.game_picker = Some(app::render::graphics::kitty_picker(8, 16));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = honor;
    st
}

/// The pane's rows as chars — searched row by row rather than as one joined
/// string, because a kitty placeholder is four bytes and a byte offset is not a
/// column.
fn rows(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width).map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' '))).collect()
        })
        .collect()
}

/// The ground a cell will actually SHOW: a reversed cell swaps the pair at draw
/// time, so its ground is the fg it stores. The reversed rows are the game's own
/// bands (`InvisiClues (tm)`, `THE CHURCHYARD`) and must stay reversed.
fn shown_ground(cell: &ratatui::buffer::Cell) -> Color {
    if cell.modifier.contains(Modifier::REVERSED) { cell.fg } else { cell.bg }
}

fn rgba_to_color(c: image::Rgba<u8>) -> Color {
    Color::Rgb(c.0[0], c.0[1], c.0[2])
}

/// The frame's own signature, asserted before anything is measured on it: a
/// release that stopped answering this route, or an arm that stopped being the
/// takeover, must fail rather than pass vacuously.
fn guard_shape(st: &app::state::AppState, buf: &Buffer, area: Rect, tag: &str) {
    assert_eq!(
        st.v6_path_log.borrow().last().map(|(l, _)| l.clone()),
        Some("painted (hint/menu takeover)".into()),
        "{tag}: the hint menu has no story window, so hybrid takes the painted takeover"
    );
    let lines = rows(buf, area);
    for want in LEGEND {
        assert!(
            lines.iter().any(|r| r.contains(want)),
            "{tag}: {want:?} must be on the hint menu, drawn with glyphs:\n{}",
            lines.join("\n")
        );
    }
}

/// **The report.** Every cell of the hint screen shows the MACHINE's page, and
/// the game's reversed bands show its ink — those two colours and nothing else.
#[test]
fn the_hint_screen_is_one_page_and_not_a_row_of_islands() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some(s) = hint_menu() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint menu");
        return;
    };
    let model = s.screen();
    let mut checked = 0usize;
    for &(w, h) in PANES {
        let st = state(true);
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
        let tag = format!("{FIXTURE} r{RELEASE} {w}x{h}");
        guard_shape(&st, &buf, area, &tag);

        let (ink, page) = app::render::screen::v6_host_pair(&st);
        let (ink, page) = (rgba_to_color(ink), rgba_to_color(page));
        assert_ne!(page, ink, "{tag}: the machine pair must have two channels");
        // The defect, exactly: a cell the runs did NOT stamp carried the page while a
        // cell they did carried the theme, so a row was as many islands as it had
        // words. Report the first offender with its column, not a count.
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buf.cell((x, y)).expect("cell in the pane");
                let ground = shown_ground(cell);
                assert!(
                    ground == page || ground == ink,
                    "{tag}: cell ({x},{y}) {:?} shows {ground:?}, which is neither the machine's \
                     page {page:?} nor its ink {ink:?} — the hint screen is the game's whole page, \
                     and a cell on another ground is an island",
                    cell.symbol()
                );
            }
        }
        // …and only the game's own REVERSED bands may be on the ink. Everything else
        // is the page, so the screen reads as one panel rather than as banded rows.
        for y in 0..area.height {
            let on_ink = (0..area.width)
                .filter(|&x| shown_ground(buf.cell((x, y)).unwrap()) == ink)
                .count();
            assert!(
                on_ink == 0 || on_ink == area.width as usize,
                "{tag}: row {y} shows the ink on {on_ink} of {} cells — a reversed band is the \
                 whole row or none of it",
                area.width
            );
        }
        checked += 1;
    }
    assert!(checked == PANES.len(), "every pane measured");
}

/// The gate: a page the INTERPRETER paints with is still a game colour, so with
/// colours declined it must not reach this screen at all.
#[test]
fn colours_declined_keeps_the_machine_page_off_the_hint_screen() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some(s) = hint_menu() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint menu");
        return;
    };
    let model = s.screen();
    // The pair the honoured render would use, resolved once from a state that
    // honours colours — the value the declined frame must not contain.
    let (ink, page) = app::render::screen::v6_host_pair(&state(true));
    let machine_page = rgba_to_color(page);
    let machine_ink = rgba_to_color(ink);

    for &(w, h) in PANES {
        let st = state(false);
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
        let tag = format!("{FIXTURE} r{RELEASE} {w}x{h} honor=false");
        guard_shape(&st, &buf, area, &tag);
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buf.cell((x, y)).unwrap();
                assert_ne!(cell.bg, machine_page, "{tag}: cell ({x},{y}) took the machine's page");
                assert_ne!(cell.fg, machine_ink, "{tag}: cell ({x},{y}) took the machine's ink");
            }
        }
    }
}

/// The two pixel modes draw ONE screen: the ground hybrid stamps into its cells
/// is the ground raster composes into its canvas, and raster's canvas holds those
/// two colours and nothing else.
///
/// Asserted as a RELATION between the modes rather than as a particular shade —
/// the machine's colour NUMBER is pinned in `zvm::interpreter`, and this case is
/// about the two paths agreeing.
#[test]
fn raster_and_hybrid_draw_the_same_page() {
    let present = stories_dir().join(FIXTURE).exists();
    let Some(s) = hint_menu() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint menu");
        return;
    };
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("{FIXTURE}: a v6 frame is Layered") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    assert!(layout.story.is_none(), "{FIXTURE}: the hint menu withdraws the story window");

    // Render the hybrid frame FIRST: `render_story_pane_frame` is what publishes
    // this frame's machine pair into the state, and `v6_host_pair` reads it back.
    // Asking before the frame has run answers with the host's own default pair,
    // and the two modes would then be compared against a pair neither drew.
    let st = state(true);
    let area = Rect::new(0, 0, 100, 34);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
    guard_shape(&st, &buf, area, &format!("{FIXTURE} hybrid-vs-raster"));

    let (ink, page) = app::render::screen::v6_host_pair(&st);
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &st);
    let mut census: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
    for px in canvas.pixels() {
        *census.entry(px.0).or_default() += 1;
    }
    assert_eq!(
        census.keys().copied().collect::<Vec<_>>(),
        {
            let mut want = vec![ink.0, page.0];
            want.sort_unstable();
            want
        },
        "{FIXTURE}: the raster canvas of a text-only screen is the machine's two colours and \
         nothing else — {} distinct colours found",
        census.len()
    );

    // …and hybrid's cells are drawn from exactly that pair (the case above proves
    // it cell by cell; this is the join between the two modes).
    let grounds: std::collections::BTreeSet<String> = (0..area.height)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .map(|(x, y)| format!("{:?}", shown_ground(buf.cell((x, y)).unwrap())))
        .collect();
    let want: std::collections::BTreeSet<String> =
        [format!("{:?}", rgba_to_color(ink)), format!("{:?}", rgba_to_color(page))].into_iter().collect();
    assert!(
        grounds.is_subset(&want),
        "{FIXTURE}: hybrid must ground every cell in the same pair raster composes with; got {grounds:?}"
    );
}
