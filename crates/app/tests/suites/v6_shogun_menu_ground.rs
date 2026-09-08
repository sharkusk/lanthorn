//! Shogun's credits/menu screen keeps its side panels and its ground in HYBRID —
//! SQ-0886.
//!
//! The report, comparing an official Amiga screenshot with ours on the same build:
//! one keypress past the splash, the ornate gold-and-black side panels are gone and
//! a full-width black block covers the top half of the screen where the machine's
//! grey ground belongs. Raster mode, on the same frame and from the same bytes, was
//! already right.
//!
//! MEASURED, before the fix, `James Clavell's Shogun.adf` (release 295, serial
//! 890321) at a 100x40 pane under kitty: `#000000` across 761 of 800 columns on
//! device rows 18..179 and not one image placement on the screen, against the
//! original's `#424542` — the Amiga palette's colour 12 — dominating every row with
//! the panels down both edges.
//!
//! WHAT IT WAS. Not the border reader, and not a fill painted over the panels: the
//! hybrid ring never ran at all. A v6 screen that prints chrome text INSIDE the
//! story window's box is classed a painted menu takeover and routed to the all-text
//! CELL path (SQ-0484, so Shogun's boot menu stops being split between the pixel
//! ring and the terminal overlay). That path draws no art whatsoever, so the panels
//! were never composed and the story window's page flooded the pane. SQ-0886
//! keeps the escape and changes its DESTINATION for one case: a takeover screen
//! with the game's own ARTWORK behind it.
//!
//! WHERE THAT DESTINATION IS NOW. SQ-0886 sent it to the composite, which draws every
//! pixel at the game's own coordinates and was the frame raster mode already shipped.
//! SQ-0892 sends it to the RING instead, which draws the panels as art and the
//! credits and menu as GLYPHS — SQ-0750's rule, and the one thing the composite
//! structurally cannot do. The two halves are pinned separately below: section 2 that
//! the artwork reaches the pane, section 3 that the text does, both across all three
//! renditions and all three panes.
//!
//! THREE RENDITIONS, because the medium is incidental — the same screen fails the
//! same way from an Amiga floppy, an IBM Blorb and a five-volume ProDOS set, and a
//! fix measured on one proves nothing about the others (CLAUDE.md). Each is named
//! by its exact release. `stories/` is gitignored, so every case skips vacuously.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One press of Shogun, by the exact build it carries.
struct Rendition {
    /// The file a person names to open it — for the ProDOS set, any one volume.
    file: &'static str,
    release: u16,
    serial: &'static str,
}

/// The three presses in the corpus. The `.po` in `stories/` is a different (and
/// unbootable) image and is deliberately not here.
const RENDITIONS: &[Rendition] = &[
    Rendition { file: "James Clavell's Shogun.adf", release: 295, serial: "890321" },
    Rendition { file: "shogun-r322-s890706.z6", release: 322, serial: "890706" },
    // The five-volume 5.25-inch ProDOS press (SHOGUN.1…SHOGUN.5); `app::hints`
    // mounts the whole set from whichever volume is named.
    Rendition { file: "shogun_s1.dsk", release: 311, serial: "890510" },
];

use crate::fixture_paths::fixture_path;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile and the artwork both come from
/// the medium — after checking the build is the one this file measured.
fn boot(r: &Rendition) -> Option<GameSession> {
    let path = fixture_path(r.file);
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
    // One keypress past the title splash is the reported screen: the credits, the
    // prompt and the START / RESTORE / QUIT menu.
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    Some(s)
}

#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn render_state(mode: app::config::V6RenderMode, honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // Halfblocks is the protocol, which is what lets a case assert on the pane's
    // own CELLS: the image lands in them.
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    state
}

/// Render one frame and hand back the pane and the path the render took.
fn frame(session: &GameSession, mode: app::config::V6RenderMode, honor: bool, pane: (u16, u16)) -> (Buffer, String) {
    let model = session.screen();
    let state = render_state(mode, honor);
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    (buf, path)
}

/// Every colour a row shows, left to right, as `(bg, fg)` per cell — the pane as a
/// player sees it, whichever half of a half-block carries the ink.
fn row_colours(buf: &Buffer, y: u16, w: u16) -> Vec<(ratatui::style::Color, ratatui::style::Color)> {
    (0..w).map(|x| (buf[(x, y)].bg, buf[(x, y)].fg)).collect()
}

/// Is this colour the game's own ink rather than a grey? The story page, the theme
/// backdrop and the text are all neutral on this screen in every press, so a
/// channel spread of any size is artwork.
fn is_chromatic(c: &ratatui::style::Color) -> bool {
    let ratatui::style::Color::Rgb(r, g, b) = *c else { return false };
    let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
    hi - lo >= 24
}

/// The premise: this really is the credits/menu screen. The three items are printed
/// through a 1px caret window one glyph at a time, so they are matched as runs.
fn is_menu_screen(session: &GameSession) -> bool {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { return false };
    let mut seen = String::new();
    for it in items {
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                seen.push_str(&t.text);
            }
        }
    }
    seen.contains("START") && seen.contains("RESTORE") && seen.contains("QUIT")
}

/// Pane sizes swept, at an 8x18 cell.
///
/// The last two are BELOW scale 1 and SQ-0898 added them, because the first three
/// are 1.25, 1.0 and 1.9875 and nothing here had ever been asked a question that
/// only a minifying pane can answer. A run is positioned through the ring scale
/// and then advances one terminal COLUMN per character, so above 1 a group of
/// glyphs under-runs its own native span and below 1 it over-runs it — and the
/// menu lost its last character to a blank the game painted after it on every
/// pane in the second regime and none in the first. `(76, 46)` is the user's own
/// 78x49 terminal; `(78, 26)` is the same regime at a different aspect, so a
/// single accident of arithmetic cannot cover both.
const PANES: &[(u16, u16)] = &[(100, 40), (80, 30), (159, 61), (76, 46), (78, 26)];

// ── 1. The reported symptom, stated directly ────────────────────────────────

/// The game's own COLOURED artwork reaches the pane.
///
/// Every press frames this screen with decoration and none of it is grey: the Amiga
/// and IBM presses run gold-on-black filigree panels down both edges, and the Apple
/// IIgs press lays red-and-green strips above and below the credits. So the test is
/// chromatic, which is what makes it rendition-agnostic — the defect left the pane
/// carrying nothing but the story page, the theme backdrop and white text, all of
/// them grey, and not one coloured pixel of the game's own art anywhere on it.
#[test]
fn the_games_own_artwork_reaches_the_pane() {
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(session) = boot(r) else { continue };
        assert!(is_menu_screen(&session), "{}: one keypress past the splash is the credits/menu screen", r.file);
        ran += 1;
        for honor in [true, false] {
            for &(w, h) in PANES {
                let (buf, _) = frame(&session, app::config::V6RenderMode::Hybrid, honor, (w, h));
                let coloured: usize = (0..h)
                    .flat_map(|y| row_colours(&buf, y, w))
                    .filter(|(bg, fg)| is_chromatic(bg) || is_chromatic(fg))
                    .count();
                assert!(
                    coloured >= 32,
                    "{} [release {}] hybrid honor={honor} {w}x{h}: the game's artwork is not on the \
                     pane — only {coloured} of {} cells carry a colour that is not a grey. That is the \
                     reported defect: the screen came out as the story window's page and nothing else \
                     (`#000000` across 761 of 800 columns), with the decoration never composed at all.",
                    r.file,
                    r.release,
                    w as usize * h as usize
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun rendition present — every case skipped");
}

// ── 3. Hybrid draws the TEXT as text ────────────────────────────────────────

/// The other half of the screen, and the half only hybrid can get right.
///
/// SQ-0886 asserted here that hybrid and raster resolve to the SAME pane, cell for
/// cell. That was true only because hybrid took the composite: it was a check on
/// agreement between two modes rather than on either of them, and the pipeline
/// document says so (§4, "tests weaker than their names"). SQ-0892 made it
/// structurally impossible and, in doing so, made it the wrong assertion — cell-wise
/// identity with a full-frame rasterisation is exactly what SQ-0750 forbids, because
/// it means every character on the screen was drawn as pixels.
///
/// So this pins the property the parity check was standing in for, stated directly:
/// hybrid puts the credits and the menu on the pane as TEXT CELLS. The composite
/// cannot — it has no glyphs at all — which is what makes this a check on hybrid and
/// not on agreement. Section 2 above pins the artwork independently, at the same
/// three panes and three renditions, so between them the screen is whole.
///
/// It is also the falsification for SQ-0892 itself: before the run grouping, driving
/// this frame through the ring gave `SI(RT th e ga me` at a 100x40 pane, and none of
/// these strings would be found.
#[test]
fn hybrid_draws_the_credits_and_menu_as_text() {
    /// Strings every press shares — nothing carrying a release or serial number,
    /// which differ by medium (295/890321, 322/890706, 311/890510).
    const LINES: &[&str] =
        &["SHOGUN", "A Story of Japan", "All rights reserved.", "START the game", "RESTORE a saved game", "QUIT the game"];
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(session) = boot(r) else { continue };
        ran += 1;
        for honor in [true, false] {
            for &(w, h) in PANES {
                let (hy, path) = frame(&session, app::config::V6RenderMode::Hybrid, honor, (w, h));
                assert_eq!(
                    path, "hybrid-ring",
                    "{} [release {}] hybrid honor={honor} {w}x{h}: a menu takeover over the game's own \
                     artwork takes the RING (SQ-0892). The cell path (`cell — painted menu takeover \
                     routed here`) loses every pixel the game drew; the composite draws every \
                     character as pixels.",
                    r.file,
                    r.release
                );
                let rows: Vec<String> = (0..h)
                    .map(|y| (0..w).map(|x| hy[(x, y)].symbol().chars().next().unwrap_or(' ')).collect())
                    .collect();
                for line in LINES {
                    assert!(
                        rows.iter().any(|row| row.contains(line)),
                        "{} [release {}] honor={honor} {w}x{h}: {line:?} is on the pane as TEXT — \
                         whole, in one row, not scattered across independently rounded cells:\n{}",
                        r.file,
                        r.release,
                        rows.join("\n")
                    );
                }
                // The three menu items keep the game's own consecutive rows.
                let row_of = |s: &str| rows.iter().position(|row| row.contains(s)).expect("asserted above");
                let start = row_of("START the game");
                assert_eq!(
                    (row_of("RESTORE a saved game"), row_of("QUIT the game")),
                    (start + 1, start + 2),
                    "{} [release {}] honor={honor} {w}x{h}: the menu keeps the game's own row \
                     order and spacing:\n{}",
                    r.file,
                    r.release,
                    rows.join("\n")
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun rendition present — every case skipped");
}

// ── 4. …and nothing else moved ──────────────────────────────────────────────

/// A takeover screen with NO artwork behind it keeps the all-text cell path.
///
/// The escape SQ-0484 added is narrowed, not removed: advent.z6 paints its boot
/// popup over a story window in a game with no artwork in it anywhere, and that
/// screen must still render as one coherent all-text page. `draw_erase_fills` draws
/// its painted panel; there are no pixels for a composite to add.
#[test]
fn a_menu_takeover_with_no_art_still_takes_the_cell_path() {
    let path = fixture_path("advent.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_win, None, None)
        .expect("advent.z6 boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // advent's boot pops a panel over the story on the first turn.
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    let (_, path) = frame(&s, app::config::V6RenderMode::Hybrid, true, (100, 40));
    assert!(
        path.starts_with("cell"),
        "advent's boot popup has no artwork behind it, so it keeps the coherent all-text path \
         SQ-0484 put it on — got {path:?}"
    );
}

// ── 4. No stranded reverse-video block past a menu item (SQ-0900) ───────────

/// The columns a menu row PAINTS are contiguous.
///
/// Shogun prints each item through a 1px caret window one glyph at a time, in
/// reverse video, and finishes with a reverse-video SPACE — so the row's painted
/// extent is the item plus one cell and nothing else. That space is a held-back
/// blank: `merge_strip_fragments` cannot tell a word space from a field gap until
/// the next inked run arrives, and a TRAILING blank never gets one, so it used to be
/// flushed as its own run. Emitted alone it is positioned by its own native x
/// through the ring's scale while the group beside it advances one terminal column
/// per character, and the two rates only agree where a terminal cell is one native
/// 8px cell — so above 85 columns the space stranded past the item and the gap
/// widened as the pane grew. MEASURED at 129x60 on release 322: the group ended at
/// column 62 and the loose space landed at column 70.
///
/// Asserted as CONTIGUITY rather than as a column number, because the item's own
/// width is an implementation detail of the scale and the gap is the defect. Both
/// `honor_game_colours` modes and every pane in `PANES`, which straddles scale 1 —
/// below it the same run produced the opposite symptom, landing on the group's last
/// cell and erasing the `e` of "game" (SQ-0898). A fix for one must not reintroduce
/// the other, so both regimes are swept here.
#[test]
fn a_menu_row_paints_one_contiguous_stretch() {
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(session) = boot(r) else { continue };
        if !is_menu_screen(&session) {
            continue;
        }
        for &pane in PANES {
            for honor in [true, false] {
                let (buf, _) = frame(&session, app::config::V6RenderMode::Hybrid, honor, pane);
                // The rows carrying the items, found by their own text.
                for y in 0..pane.1 {
                    let row: String = (0..pane.0).map(|x| buf[(x, y)].symbol().to_string()).collect();
                    if !(row.contains("START the") || row.contains("RESTORE a") || row.contains("QUIT the")) {
                        continue;
                    }
                    // The defect is a BLANK glyph carrying a painted background,
                    // sitting away from the item — the reverse-video space. The
                    // flank artwork also paints this row on the Amiga and IBM
                    // presses, but it paints half-block GLYPHS (`▀▄`), so keying on
                    // a blank symbol separates the bar from the border rather than
                    // fighting it.
                    // The reverse-video bar is a MODIFIER, not a colour: measured on
                    // this row, `bg` is one uniform grey across the whole menu panel
                    // (columns 8..=91 at a 100x40 pane) and only `Modifier::REVERSED`
                    // marks the item. Keying on it separates the bar from the flank
                    // artwork and the panel page in one step.
                    use ratatui::style::Modifier;
                    let rv: Vec<u16> = (0..pane.0)
                        .filter(|&x| buf[(x, y)].modifier.contains(Modifier::REVERSED))
                        .collect();
                    // Only the SELECTED item carries the bar; the other two rows have
                    // no reversed cells at all and are vacuously fine.
                    if rv.is_empty() {
                        continue;
                    }
                    let (first, last) = (rv[0], rv[rv.len() - 1]);
                    assert_eq!(
                        rv.len() as u16,
                        last - first + 1,
                        "{} [release {}] at {}x{}, honor={honor}, row {y}: the reverse-video \
                         columns are {rv:?} — not one contiguous bar from {first} to {last}. The \
                         hole then a lone reversed cell is the item's trailing space placed by \
                         its own native x instead of by the group it belongs to (SQ-0900).\n \
                         row: {row:?}",
                        r.file,
                        r.release,
                        pane.0,
                        pane.1,
                    );
                    ran += 1;
                }
            }
        }
    }
    if fixture_path(RENDITIONS[1].file).exists() {
        assert!(ran > 0, "the fixtures are present but no menu row was found — check the premise");
    }
}
