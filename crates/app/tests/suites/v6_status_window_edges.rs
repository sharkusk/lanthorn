//! Where a v6 status WINDOW ends, and what the hybrid ring may draw there —
//! SQ-0948 and SQ-0949, which are one defect seen from its two sides.
//!
//! A v6 frame puts a wide status window between two columns of border artwork, and
//! its edge almost never falls on a terminal cell boundary. Hybrid draws the window
//! with CELLS and the artwork with an IMAGE, so that one native column is claimed
//! twice — and whichever of the two claims it paints its own ground over the other's
//! pixels. The two reports are the two directions:
//!
//! * **SQ-0948, the band reaching in.** `stories/shogun-r322-s890706.z6` (release
//!   322, serial 890706, IBM PC), six taps into play at a 117x40 kitty terminal. The
//!   status window is `548x32` at native `(46, 0)`; the left flank band's last
//!   terminal cell inverts to native `44.5..50.1`, so four columns of the window's
//!   PAGE sat inside the flank's own image. The strip drew that ribbon 36 device px
//!   tall — two whole cells — while the band drew the same page 46 px tall, its true
//!   32 native rows, and the 10-pixel difference reached the screen as a 6x10 white
//!   block hanging below each end of the score bar. The user reported it twice; the
//!   first time it was dismissed as a harness artifact, which it was not.
//!
//! * **SQ-0949, the strip reaching out.** `stories/arthur-r74-s890714.z6` (release
//!   74, serial 890714, IBM PC, art from the Blorb beside it), twelve taps in. Its
//!   status window is `584x16` at native `(28, 192)` — 91% of the screen, wide enough
//!   that `full_width_flood_rows` calls the row a bar, and 28 native columns short of
//!   each edge, which is exactly where Arthur's poles stand. The flank veto read "a
//!   bar owns the row" as "a bar owns the PANE", so the strip's ground flooded across
//!   both poles and the ring carved each flank into a piece above the ribbon and a
//!   piece below it. That seam is the step the report calls the side strip not lining
//!   up with the panel above it.
//!
//! **The reference machines settle which side is right**, and neither shows a bar
//! that reaches the screen's edge. `machine-screenshots/dos-arthur.png` — the EGA
//! press at the Churchyard, "Merlin disappears as suddenly as he came" — puts the
//! white ribbon at native 28..610 and the grey pole rule beside it at native
//! 6.5..8.7, unbroken from the panel's foot to the bottom of the screen.
//! `machine-screenshots/mac-arthur.png` is the same moment with the black ribbon
//! inset and the green poles at one constant x above and below it.
//!
//! Both stories are gitignored, so every case here skips cleanly when absent.
//!
//! **Colour mode**: both `honor_game_colours` settings are swept. The page fill
//! SQ-0948 turns on is gated on that flag, so a single-mode case would pin only half
//! of it.
//!
//! **Palette: stated, not inherited** (SQ-0958, SQ-1393). The boot carries the IBM
//! PC's own table and the scheme every case renders through is built from the same
//! one, so nothing here reads a table it did not name.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the mount
/// returned, and the screen size through the whole `std_window → native_std_window →
/// profile` chain — then tap `taps` times, answering `n` to a restore question.
///
/// A harness that skips `native_std_window` measures a screen the player never sees
/// (CLAUDE.md); both fixtures here are 320x200 presses drawn at art scale (2,2), so
/// the chain matters even though neither is a disk image.
fn boot(name: &str, keys: u8, taps: usize) -> Option<GameSession> {
    let path = stories_dir().join(name);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
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
        // The IBM PC's own table, which is what all three cases here measure
        // through — stated on the boot rather than installed around the case
        // (SQ-1393).
        InterpreterProfile::IbmPc.palette(),
        None,
    );
    let mut session = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
    .expect("the story should boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(keys).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(session)
}

/// The chrome GRID window that carries `needle`, as native `(x, y, w, h)` — the
/// status window, found by its own text rather than by a hard-coded rect.
fn status_window(model: &app::engine::ScreenModel, needle: &str) -> Option<(u16, u16, u16, u16)> {
    let WinNode::Layered(items) = &model.root else { return None };
    items.iter().find_map(|pw| {
        let WinNode::Grid(g) = &pw.node else { return None };
        // Joined, because a game with proportional metrics emits one run per GLYPH —
        // Arthur's band arrives as `C` `h` `u` … and no single run contains the word.
        let joined: String = g.px_texts.iter().map(|t| t.text.as_str()).collect();
        joined.contains(needle).then_some((pw.x_px, pw.y_px, pw.w_px, pw.h_px))
    })
}

/// A hybrid render at a real kitty cell (8x18). `Picker::halfblocks()` reports a 1x2
/// cell, a layout regime in which no sub-cell boundary exists to fall on.
#[allow(deprecated)]
fn render(model: &app::engine::ScreenModel, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default_in(InterpreterProfile::IbmPc.palette());
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// A cell the ring drew as part of an uploaded band: a kitty virtual placeholder.
fn is_band_cell(c: &ratatui::buffer::Cell) -> bool {
    c.symbol().starts_with('\u{10eeee}')
}

// ── SQ-0948: a flank band may not carry a page the cells draw ────────────────

/// Shogun's chrome canvas carries NO pixel of the status window's page on the rows
/// the ring draws with glyphs — so the flank band beside it cannot ship one.
///
/// The band is the only thing that draws at sub-cell resolution, so a page it carries
/// is a second rendering of a ribbon the strip has already drawn: at the window's true
/// 32 native rows rather than the strip's two whole cells, and the ten device pixels
/// of difference are the block hanging below each end of the score bar. Measured on
/// the canvas the bands are cut from, since the cell layer cannot tell a page inside a
/// band from artwork.
///
/// Self-falsifying: the same canvas built with `TextLayer::All` — what raster wants,
/// and what hybrid used to pass — must still carry the page, or this case is asserting
/// nothing.
#[test]
fn shoguns_chrome_canvas_keeps_the_status_page_off_the_ring() {
    let Some(session) = boot("shogun-r322-s890706.z6", 13, 6) else { return };
    let model = session.screen();
    let Some((sx, sy, sw, sh)) = status_window(&model, "Score:") else {
        panic!("six taps in, Shogun's status window carries `Score:` — the frame this case is about")
    };
    // The shape this case depends on: a status window whose left edge is native 46,
    // which at a 115-column pane is column 8.27 and so falls INSIDE the flank's last
    // cell. Without this guard a frame that had moved the window would pass vacuously.
    assert_eq!(
        (sx, sy, sw, sh),
        (46, 0, 548, 32),
        "the specimen is the status window at native (46,0) 548x32; its edge landing \
         inside a terminal cell is the whole of SQ-0948"
    );

    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    let layout = app::render::v6_layout::classify_windows(items.as_slice(), zvm::screen::V6Cell::DEFAULT);
    // The rows the ring claims: Shogun prints its band at native y 1 and 17, so their
    // tops are 0 and 16 and between them they are the window's whole 32 rows.
    let glyph_rows: std::collections::HashSet<u16> = layout
        .chrome
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().map(|t| t.y.max(1) - 1)),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        glyph_rows.contains(&0) && glyph_rows.contains(&16),
        "the ring draws both of the band's native rows with glyphs; got {glyph_rows:?}"
    );

    // The game's own erase of that window, as the app records it. MEASURED from the
    // running binary two turns in: `state.v6_paint` is a 548x368 surface — sized to
    // the STORY window — carrying the status window's fill in SCREEN coordinates, so
    // the rectangle the game erased at native (46,0) 548x32 is clipped at native 548
    // and never reaches the right flank. That asymmetry is why the page half of this
    // fix cured one side of the score bar and left the other.
    let mut paint = image::RgbaImage::new(548, 368);
    for y in 0..32u32 {
        for x in 46..548u32 {
            paint.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }

    let colors = app::colors::ColorScheme::terminal_default_in(InterpreterProfile::IbmPc.palette());
    let build = |text: app::render::v6_layout::TextLayer<'_>| {
        let mut canvas = app::render::v6_layout::build_chrome_canvas(
            &layout.chrome,
            native,
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 0, 255]),
            &colors,
            text,
            &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT),
        );
        // The ring's own order (SQ-0706): art and glyphs, then the painted ground
        // beneath them, then the window pages filling whatever neither claimed.
        app::render::v6_layout::blit_paint_ground(&mut canvas, Some(&paint), text, zvm::screen::V6Cell::DEFAULT);
        app::render::v6_layout::fill_window_pages(&mut canvas, &layout.chrome, layout.story, &colors, text, zvm::screen::V6Cell::DEFAULT);
        canvas
    };
    // The four native columns the flank's last cell reaches into, over the window's
    // own rows — the exact pixels that became the white block.
    let leak = |canvas: &image::RgbaImage| -> usize {
        (u32::from(sy)..u32::from(sy) + u32::from(sh))
            .flat_map(|y| (u32::from(sx)..u32::from(sx) + 4).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.get_pixel(x, y)[3] != 0)
            .count()
    };

    let hybrid = build(app::render::v6_layout::TextLayer::SkipGlyphRows(&glyph_rows));
    assert_eq!(
        leak(&hybrid),
        0,
        "on a row the ring draws with GLYPHS the canvas keeps artwork and nothing else \
         (SQ-0750/SQ-0903), and neither a window PAGE nor the ground the game erased \
         under a ribbon is artwork — the strip stamps that ground into its own cells, \
         so a band carrying it draws the same ribbon a second time at the window's \
         true height"
    );
    let raster = build(app::render::v6_layout::TextLayer::All);
    assert_eq!(
        leak(&raster),
        (sh as usize) * 4,
        "…and the raster composite, which has no cells to draw the ribbon with, must \
         still get every pixel of that page — if it does not, the case above is \
         measuring a canvas that never had one"
    );
}

// ── SQ-0949: a ribbon reaches as far as its own window and no further ────────

/// Arthur's poles run through the status row unbroken, and the ribbon stops short of
/// them — at every pane, in both colour modes.
///
/// The pole is native columns 6..10 (left) and 630..634 (right) of 640; the status
/// window is native 28..612. So the flank keeps its columns on the ribbon's row, and
/// the band that draws them is ONE piece spanning the rows above and below it rather
/// than two with the ribbon between.
///
/// FALSIFY by dropping the `row_spans` clause from `ChromeRowOracle::blocked`: the
/// ribbon takes the whole pane, the flank's cells on that row become the strip's
/// ground, and the assertion below reports the row as bare at both edges.
#[test]
fn arthurs_poles_run_through_the_status_row_unbroken() {
    let Some(session) = boot("arthur-r74-s890714.z6", 13, 12) else { return };
    let model = session.screen();
    let Some((sx, sy, sw, sh)) = status_window(&model, "Churchyard") else {
        panic!(
            "twelve taps in, Arthur is at the Churchyard and his status window carries its \
             name — the frame this case is about"
        )
    };
    assert_eq!(
        (sx, sy, sw, sh),
        (28, 192, 584, 16),
        "the specimen is the status window at native (28,192) 584x16 — 91% of the \
         screen, so it reads as a BAR, and 28 native columns short of each edge, which \
         is where the poles are"
    );

    let mut checked = 0usize;
    for honor in [true, false] {
        for (cols, rows) in [(115u16, 37u16), (98, 37), (143, 47), (80, 25)] {
            let (area, buf) = render(&model, honor, cols, rows);
            let row_text = |y: u16| -> String {
                (0..area.width)
                    .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                    .collect()
            };
            let Some(status_y) = (0..area.height).find(|&y| row_text(y).contains("Anne")) else {
                panic!("{cols}x{rows} honor={honor}: the status date must reach the cell layer")
            };
            let where_ = format!("{cols}x{rows} honor={honor}");

            // The frame's own columns on the ribbon's row still belong to the ring.
            let band_cols: Vec<u16> = (0..area.width)
                .filter(|&x| is_band_cell(buf.cell((x, status_y)).unwrap()))
                .collect();
            assert!(
                band_cols.first().is_some_and(|&c| c == 0),
                "{where_}: the ribbon stops at its window, so the pane's first column on \
                 the status row is still the flank's band\nrow: {:?}",
                row_text(status_y)
            );
            assert!(
                band_cols.last().is_some_and(|&c| c + 1 == area.width),
                "{where_}: …and so is its last\nrow: {:?}",
                row_text(status_y)
            );

            // …and the pole is CONTINUOUS across the seam: the same column carries the
            // ring's band on the row above the ribbon, on the ribbon's own row, and on
            // the row below. That is the step the report describes.
            for y in [status_y - 1, status_y, status_y + 1] {
                assert!(
                    is_band_cell(buf.cell((0, y)).unwrap()),
                    "{where_}: column 0 must be the flank's band on rows {}..={} — the pole \
                     runs from the panel's foot to the bottom of the screen without the \
                     ribbon cutting it in two\nrow {y}: {:?}",
                    status_y - 1,
                    status_y + 1,
                    row_text(y)
                );
            }

            // The ribbon itself is intact: the room name survived the strip's narrower
            // rect rather than being dropped with the padding cell in front of it.
            assert!(
                row_text(status_y).contains("Churchyard"),
                "{where_}: the location must still read as one word\nrow: {:?}",
                row_text(status_y)
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 8, "every pane and colour mode was measured");
}

// ── SQ-0967: the painted ground is a SCREEN, not the story window ────────────

/// Shogun's status erase reaches the RIGHT flank on the painted ground.
///
/// The third layer of the same defect. `erase_window` records its rectangle in
/// SCREEN coordinates (`GameSession::apply_erase_fill`), but the surface it is
/// recorded into was allocated at window 0's box — the STORY window — so every
/// pixel past that box was silently dropped. Shogun r322 erases native (46,0)
/// 548x32 for its score bar, reaching to x=594, on a 548x368 surface: the fill
/// stopped dead at 548 and never reached the right flank at native 590. SQ-0948
/// fixed the declared page and cured the right-hand block; the left one survived
/// on this layer, which is what cost that lane a round.
///
/// Not Shogun-specific — the ProDOS presses allocate short too (Journey r77:
/// 304x288 for a 560x384 screen; Shogun r311: 560x320) — but Shogun r322 is where
/// the overhang has pixels in it, so it is the specimen.
///
/// **Frame**: six taps of Enter, the same gameplay frame the SQ-0948 case above
/// uses, guarded on the same window rect.
///
/// **Palette**: stated, not inherited (SQ-0958) — the fill's colour is one the game
/// named outright, resolved through the profile's own table.
///
/// No colour-mode sweep: this is the engine's own surface, built from the colour
/// the game named (`explicit_pixel_rgba`) with no `ColorScheme` in the path, so
/// `honor_game_colours` cannot reach it. The two cases above sweep the ring that
/// consumes it.
#[test]
fn shoguns_status_erase_reaches_past_the_story_window_on_the_painted_ground() {
    let Some(session) = boot("shogun-r322-s890706.z6", 13, 6) else { return };
    let model = session.screen();
    let Some((sx, sy, sw, sh)) = status_window(&model, "Score:") else {
        panic!("six taps in, Shogun's status window carries `Score:` — the frame this case is about")
    };
    assert_eq!(
        (sx, sy, sw, sh),
        (46, 0, 548, 32),
        "the specimen is the status window at native (46,0) 548x32"
    );

    // The premise, and the whole of the defect: the window the ground USED to be cut
    // to is narrower than the rectangle the game erased. Without this the case would
    // pass on a frame where nothing hangs over and prove nothing.
    let w0 = session.machine.screen.v6.as_ref().map(|v6| {
        let w = &v6.windows[0];
        (u32::from(w.x_size), u32::from(w.y_size))
    });
    assert_eq!(w0, Some((548, 368)), "window 0 is Shogun's STORY window, not its screen");
    let right = u32::from(sx) + u32::from(sw);
    assert!(
        right > 548,
        "the status erase must reach past window 0's 548 columns — it ends at {right}"
    );

    let ground = Engine::paint_surface(&session).expect("the score bar's erase paints a ground");
    // The reported symptom first, so reverting the fix fails on it rather than on the
    // allocation it is a consequence of. A pixel off the surface counts as bare, which
    // is precisely what clipping did to it.
    let opaque = |x: u32, y: u32| {
        x < ground.width() && y < ground.height() && ground.get_pixel(x, y)[3] != 0
    };

    // Every pixel of the erased rectangle is on the ground — including the 46 columns
    // past window 0's edge, where the score bar's right end lives.
    let painted = (0..u32::from(sh))
        .flat_map(|y| (u32::from(sx)..right).map(move |x| (x, y)))
        .filter(|&(x, y)| opaque(x, y))
        .count();
    assert_eq!(
        painted,
        usize::from(sw) * usize::from(sh),
        "the whole 548x32 fill reaches the ground; clipped at window 0's 548 columns it \
         is {} pixels short",
        (right - 548) as usize * usize::from(sh)
    );

    // …and the reported symptom itself: native 590 is inside the RIGHT flank, 42
    // columns past where the surface used to end.
    let overhang: Vec<(u32, u32)> =
        (0..u32::from(sh)).map(|y| (590u32, y)).filter(|&(x, y)| !opaque(x, y)).collect();
    assert!(
        overhang.is_empty(),
        "the ribbon's ground must cover native x=590 on all {sh} of its rows — the flank \
         is exactly what a surface cut to the story window could not represent (SQ-0967); \
         bare at {overhang:?}"
    );
    assert_eq!(
        *ground.get_pixel(590, 16),
        image::Rgba([173, 173, 173, 255]),
        "and it is the colour the game named for its score bar, resolved through the IBM \
         PC table — §8.3.1 white is 255,255,255 and this press's is 173,173,173, which is \
         the difference a stated palette makes (SQ-0958)"
    );

    // The allocation those pixels depend on: the ground shares a coordinate space AND
    // an extent with the chrome canvas `blit_paint_ground` copies it into.
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items.as_slice(), &app::native_font::TextFace::cell_only(zvm::screen::V6Cell::DEFAULT));
    assert_eq!(
        (ground.width(), ground.height()),
        (u32::from(native.0), u32::from(native.1)),
        "the painted ground is the size of the screen the ring composites onto, not of \
         one window inside it"
    );
    assert_eq!((ground.width(), ground.height()), (640, 400), "Shogun r322 is an IBM PC press");
}
