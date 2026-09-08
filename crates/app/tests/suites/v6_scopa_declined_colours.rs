//! SQ-0716: declining the game's colours must not delete half of a drawing.
//!
//! scopa's felt table used to vanish under `honor_game_colours = false`, leaving a
//! BLACK table carrying the two green bands and the cards the game had drawn onto
//! it — the worst of both readings.
//!
//! Measured from the SCREEN OPS rather than the model, the mechanism is not "a
//! page is a preference, a fill is drawing". scopa's boot is:
//!
//! ```text
//! @set_true_colour(fg=true(0x0000), bg=true(0x0200), window=1)   # explicit green
//! @window_size(win=1, y=400, x=640)                              # the whole screen
//! @erase_window(all(unsplit))
//! @erase_window(upper)                                           # = window 1
//! ```
//!
//! The table is a FILL — the same `erase_window` opcode SQ-0706 declared ungatable
//! when it made the cards survive a declined palette. It only reaches the renderer
//! as a window *page* because `drain_erase_fills` classifies a fill spanning the
//! whole screen as a screen clear and drops it, so window 1's background is the
//! sole surviving record of the paint. Gating that record on the colour flag while
//! leaving the sub-screen fills ungated is what split one drawing in half.
//!
//! The fix keys on the painted ground, the same discriminator SQ-0711 uses: a
//! window with the game's own pixels inside it is a canvas and keeps its page
//! either way; a window with none is presentation and the flag still governs it.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.


use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::fixture_paths::fixture_path;


/// Boot scopa to its title table, optionally dealing a hand. `Engine::set_mouse`
/// takes the coordinates Y FIRST; a click reaches `read_char` as ZSCII 254
/// (ZMSD §3.8) with the coordinates already set.
fn scopa(deal: bool, trace: bool) -> Option<GameSession> {
    let path = fixture_path("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, trace, dims, picts.std_window(), None, None,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    if deal {
        for _ in 0..3 {
            match s.pending_input() {
                app::session::InputKind::Char => {
                    Engine::set_mouse(&mut s, 350, 245);
                    let _ = s.submit_char(254);
                }
                _ => {
                    let _ = s.submit("");
                }
            }
            let _ = s.take_transcript();
        }
    }
    Some(s)
}

/// scopa's baize: `set_true_colour` 0x0200 is 15-bit RGB with only G bit 4 set,
/// which resolves to `Rgb(0, 132, 0)`. Matched by shape so an anti-aliased edge
/// cell still counts.
fn is_baize(c: Color) -> bool {
    matches!(c, Color::Rgb(r, g, b) if g >= 100 && r < 90 && b < 90)
}

fn render(session: &GameSession, mode: app::config::V6RenderMode, honor: bool, w: u16, h: u16) -> Buffer {
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    *state.v6_paint.borrow_mut() = session.paint_surface();
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    buf
}

/// The premise, from the screen ops: the table is an explicit-colour full-screen
/// `erase_window`, and the painted ground does NOT carry it — so the window page
/// is the only record there is.
#[test]
fn the_table_is_a_full_screen_fill_that_survives_only_as_a_window_page() {
    let Some(session) = scopa(false, true) else { return };
    let mut session = session;
    let ops = Engine::take_screen_trace(&mut session);
    assert!(
        ops.iter().any(|l| l.contains("@set_true_colour") && l.contains("bg=true(0x0200)") && l.contains("window=1")),
        "scopa names its green outright on window 1 — an explicit colour, not an inherited one:\n{ops:#?}"
    );
    assert!(
        ops.iter().any(|l| l == "@window_size(win=1, y=400, x=640)"),
        "…and window 1 is the whole 640x400 screen, which is why its erase is dropped as a clear"
    );
    assert!(
        ops.iter().any(|l| l.starts_with("@erase_window")),
        "…and the table arrives as an erase_window fill"
    );

    // The dropped half: the painted ground has no pixel in the table's corners.
    let paint = session.paint_surface().expect("scopa paints its cards (SQ-0706)");
    assert_eq!(
        paint.get_pixel(0, 0)[3],
        0,
        "the full-screen fill is dropped as a screen clear, so the ground carries no table"
    );

    // …and window 1 still declares that green as its page.
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    let table = layout
        .chrome
        .iter()
        .find(|it| it.w_px == 640 && it.h_px == 400)
        .expect("the full-screen table window is published");
    let app::engine::WinNode::Grid(g) = &table.node else { panic!("scopa's screen is Grids") };
    assert_eq!(g.bg, Some(0x0200_0200), "the page is the surviving record of that fill (true-colour 0x0200)");
}

/// The symptom: with the game's colours declined the table must still be there.
///
/// Both render modes, both scopa screens (title and dealt) and four pane sizes —
/// the felt is resolved in native pixels, so no pane size may lose it to
/// cell quantization.
///
/// Falsified by reverting `fill_painted_window_pages` to the plain
/// `if honor { fill_window_pages(.., zvm::screen::V6Cell::DEFAULT) }` gate:
/// "Hybrid honor=false title pane=100x34: the felt table reaches the pane — 0 of
/// 3400 cells are baize, wanted more than 1700".
#[test]
fn declining_game_colours_keeps_the_felt_table() {
    for (label, deal) in [("title", false), ("dealt", true)] {
        let Some(session) = scopa(deal, false) else { return };
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            for (w, h) in [(100u16, 34u16), (80, 25), (132, 50), (64, 20)] {
                let buf = render(&session, mode, false, w, h);
                let cells = (w as usize) * (h as usize);
                let green = buf.content().iter().filter(|c| is_baize(c.bg)).count();
                assert!(
                    green > cells / 2,
                    "{mode:?} honor=false {label} pane={w}x{h}: the felt table reaches the pane — \
                     {green} of {cells} cells are baize, wanted more than {}",
                    cells / 2
                );
            }
        }
    }
}

/// SQ-0784: the score screen's row fills must not bridge the gap between two
/// separated runs — the table shows through it.
///
/// The report is the end-of-hand score screen, reproduced from a host Save State
/// taken on it: scopa prints its whole board into ONE 640x400 grid whose page is
/// the baize, and three of its rows carry two runs with a wide gap between them —
/// `Denari` (native x 154) and `Primiera` (466) at y 86, and the two Denari/Primiera
/// totals at y 145 and 239. The game draws its two blue card panels either side of a
/// green divider at native 350..360, and `fill_explicit_bg_rows` flooded each row's
/// whole HULL (153..529, 13..385) — three blue bridges straight through the divider,
/// which is what the player reported seeing.
///
/// The runs and coordinates below are the ones measured off that save; the save is
/// the player's own and not redistributable, so the pattern travels rather than the
/// file. Both `honor_game_colours` modes: the fill is a colour behaviour, and the
/// divider must survive either way (the baize is only PAINTED in when the game's
/// colours are honoured, so that is the mode that can name its colour).
///
/// The bar that MUST still bridge is pinned beside the fix
/// (`explicit_bg_status_row_floods_the_whole_window_width`) and on a real fixture by
/// `v6_shogun_gameplay::shogun_raster_status_band_floods_game_white`.
#[test]
fn the_score_screen_does_not_bridge_the_gap_between_two_runs() {
    use app::engine::{BorderPref, GridWindow, PositionedWindow, PxText, ScreenModel, StatusModel, WinNode};

    // scopa's own white-on-blue panel pair and its baize page, as the game names
    // them: 15-bit true colour, 0x59A0 -> Rgb(0,107,181) and 0x0200 -> Rgb(0,132,0).
    let ink = 0x0200_7FFF;
    let panel = 0x0200_59A0;
    let blue = image::Rgba([0, 107, 181, 255]);
    let baize = image::Rgba([0, 132, 0, 255]);
    let px = |x: u16, y: u16, s: &str| {
        PxText::derived(y, x, s.into(), 0, ink, panel, zvm::screen::V6Cell::DEFAULT)
    };
    let board = PositionedWindow {
        x: 0, y: 0, w: 80, h: 25, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
        left_margin: 0, right_margin: 0,
        node: WinNode::Grid(GridWindow {
            cols: 80, rows: 25, active_rows: 25, bg: Some(0x0200_0200),
            border: BorderPref::Unspecified,
            px_texts: vec![
                px(154, 86, "Denari"), px(466, 86, "Primiera"),
                px(14, 145, "8"), px(370, 145, "72"),
                px(14, 239, "2"), px(370, 239, "84"),
            ],
            ..Default::default()
        }),
    };
    let model = ScreenModel {
        root: WinNode::Layered(vec![board]),
        status: StatusModel::HostManaged,
        bg: 0,
        fg: 0,
        content_size: (80, 25),
    };
    let WinNode::Layered(items) = &model.root else { unreachable!() };
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Raster;
        state.config.honor_game_colours = honor;
        let (canvas, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        for (row, (left, lx), (right, rx)) in [
            (86u32, ("Denari", 153u32), ("Primiera", 465u32)),
            (145, ("8", 13), ("72", 369)),
            (239, ("2", 13), ("84", 369)),
        ] {
            // The last scanline of the 16-pixel text cell — below every glyph on the
            // row, so what it shows is the background alone.
            let y = row - 1 + 15;
            assert_ne!(
                *canvas.get_pixel(355, y), blue,
                "honor={honor} row {row} ({left} -> {right}): native 350..360 is the divider between \
                 scopa's two card panels; the row fill must not bridge {left} to {right} across it"
            );
            if honor {
                assert_eq!(
                    *canvas.get_pixel(355, y), baize,
                    "honor=true row {row}: what shows in the gap is the board window's own baize page"
                );
            }
            assert_eq!(*canvas.get_pixel(lx, y), blue, "honor={honor} row {row}: {left} keeps its own panel bg");
            assert_eq!(*canvas.get_pixel(rx, y), blue, "honor={honor} row {row}: {right} keeps its own panel bg");
        }
    }
}

/// The same invariant on the real story, so a change to how scopa publishes its
/// screen cannot quietly stop the case above from describing it: no row of any
/// chrome window may flood a bare gap between two of its runs unless that row
/// reaches both of its window's edges.
///
/// Asked of the CHROME CANVAS rather than the finished composite, because that is
/// the layer the flood belongs to — everything after it (the painted ground, the
/// window pages) legitimately colours in what is left, and a gap this row did not
/// claim is exactly what those layers are for. `build_chrome_canvas` takes no
/// `honor_game_colours`, which is why one pass covers both modes here and the
/// synthetic case above carries the two-mode composite.
///
/// It is a GUARD, not the reproduction: every row scopa reaches without playing a
/// hand out carries a single run (`Help`, `Credits`, `Quit`, `Click on a card type
/// to begin`), so this passes with the defect in place and would only start
/// failing if the game — or the way we publish its screen — ever put two separated
/// runs on a reachable row. The rows that do carry two are the score screen's, and
/// reaching those needs the player's own save.
///
/// Skips vacuously when the gitignored story is absent.
#[test]
fn no_reachable_scopa_row_floods_a_gap_it_does_not_span() {
    use app::engine::WinNode;

    for deal in [false, true] {
        let Some(session) = scopa(deal, false) else { return };
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
        let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
        let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
        let colors = app::colors::ColorScheme::terminal_default();
        let canvas = app::render::v6_layout::build_chrome_canvas(
            &layout.chrome,
            native,
            image::Rgba([200, 200, 200, 255]),
            image::Rgba([0, 0, 0, 255]),
            &colors,
            app::render::v6_layout::TextLayer::All,
            &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );

        for it in &layout.chrome {
            let WinNode::Grid(g) = &it.node else { continue };
            let (ox, win_w) = (it.x_px as u32, it.w_px as u32);
            let mut rows: std::collections::BTreeMap<u16, Vec<&app::engine::PxText>> = Default::default();
            for t in &g.px_texts {
                rows.entry(t.y).or_default().push(t);
            }
            for (y, mut runs) in rows {
                runs.sort_by_key(|t| t.x);
                let span = |t: &app::engine::PxText| {
                    let x0 = u32::from(t.x.max(1)) - 1;
                    (x0, x0 + t.text.chars().count().max(1) as u32 * 8)
                };
                let lo = runs.first().map(|t| span(t).0).unwrap_or(ox);
                let hi = runs.iter().map(|t| span(t).1).max().unwrap_or(ox);
                if lo <= ox + 8 && hi + 8 >= ox + win_w {
                    continue; // a bar: it reaches both edges, so it bridges by design
                }
                for w in runs.windows(2) {
                    let (gap0, gap1) = (span(w[0]).1, span(w[1]).0);
                    if gap1 <= gap0 {
                        continue;
                    }
                    let gx = (gap0 + gap1) / 2;
                    let gy = (u32::from(y.max(1)) - 1 + 15).min(canvas.height().saturating_sub(1));
                    assert_eq!(
                        canvas.get_pixel(gx, gy)[3],
                        0,
                        "deal={deal}: the bare gap at native x {gap0}..{gap1} on row {y} of the \
                         {win_w}px window is not this row's to paint — {:?} then {:?}",
                        w[0].text, w[1].text
                    );
                }
            }
        }
    }
}

/// Half-honoured is the thing that was wrong, so pin the whole: for a game that
/// draws its board and streams no prose, declining its colours changes nothing at
/// all. Every pixel on scopa's screen is its own drawing.
#[test]
fn scopa_looks_the_same_either_way_because_all_of_it_is_drawing() {
    for (label, deal) in [("title", false), ("dealt", true)] {
        let Some(session) = scopa(deal, false) else { return };
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            for (w, h) in [(100u16, 34u16), (80, 25)] {
                let on = render(&session, mode, true, w, h);
                let off = render(&session, mode, false, w, h);
                assert_eq!(
                    on, off,
                    "{mode:?} {label} pane={w}x{h}: scopa's screen is drawing end to end, so the \
                     colour flag has nothing left to decline"
                );
            }
        }
    }
}
